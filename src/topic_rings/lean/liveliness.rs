use std::{sync::atomic::Ordering, time::Duration};

use super::{
    gate::ACTIVE,
    liveness::{process_alive, LEAN_MAX_PRODUCERS, STATE_MASK},
    monotonic_ns, LeanChannel, LeanLivelinessKind, LeanOwnership, LeanQos, LeanRing,
};
use crate::error::{RenoirError, Result};

const OWNER_RECHECK_NS: u64 = 2_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LivelinessStatus {
    pub registered: usize,
    pub alive: usize,
    pub lost: bool,
    pub lost_count: u64,
}

#[derive(Debug, Default)]
pub(super) struct QosState {
    pub(super) lifespan_units: u64,
    pub(super) min_separation_units: u64,
    pub(super) last_delivered: Option<u64>,
    pub(super) stale_dropped: u64,
    pub(super) filtered_out: u64,
    pub(super) manual: bool,
    pub(super) lease_ns: u64,
    pub(super) was_alive: bool,
    pub(super) lost_count: u64,
    pub(super) exclusive: bool,
    pub(super) strength: u64,
    pub(super) owner_checked_ns: u64,
    pub(super) owner_cached: bool,
}

impl LeanRing {
    fn entry_alive(&self, index: usize, lease_ns: u64, now_ns: u64) -> bool {
        let entry = self.producer_entry(index);
        if entry.owner.load(Ordering::Acquire) & STATE_MASK != ACTIVE
            || !process_alive(
                entry.pid.load(Ordering::Acquire),
                entry.start.load(Ordering::Acquire),
            )
        {
            return false;
        }
        lease_ns == 0
            || now_ns.saturating_sub(entry.last_assert.load(Ordering::Acquire)) <= lease_ns
    }

    pub(super) fn liveliness_counts(&self, lease_ns: u64) -> (usize, usize) {
        let now = monotonic_ns();
        let mut registered = 0;
        let mut alive = 0;
        for i in 0..LEAN_MAX_PRODUCERS {
            if self.producer_entry(i).owner.load(Ordering::Acquire) & STATE_MASK == ACTIVE {
                registered += 1;
                if self.entry_alive(i, lease_ns, now) {
                    alive += 1;
                }
            }
        }
        (registered, alive)
    }

    pub(super) fn current_owner(&self, lease_ns: u64) -> Option<usize> {
        let now = monotonic_ns();
        let mut best: Option<(u64, usize)> = None;
        for i in 0..LEAN_MAX_PRODUCERS {
            if !self.entry_alive(i, lease_ns, now) {
                continue;
            }
            let strength = self.producer_entry(i).strength.load(Ordering::Acquire);
            if best.is_none_or(|(s, _)| strength > s) {
                best = Some((strength, i));
            }
        }
        best.map(|(_, i)| i)
    }
}

impl LeanChannel {
    pub(super) fn apply_qos(&mut self, qos: &LeanQos, subscriber: bool) {
        self.set_deadline(qos.deadline);
        let ring = self.ring().clone();
        let offered = ring.offered_lifespan_ns();
        let requested = qos
            .lifespan
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        let lifespan_ns = match (offered, requested) {
            (0, r) => r,
            (o, 0) => o,
            (o, r) => o.min(r),
        };
        let separation = qos
            .min_separation
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        let manual = qos.liveliness.kind == LeanLivelinessKind::Manual
            || (subscriber && ring.offered_manual());
        let lease_ns = if manual {
            qos.liveliness.lease.map_or_else(
                || ring.offered_lease_ns(),
                |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
            )
        } else {
            0
        };
        self.qos.lifespan_units = ring.ns_to_units(lifespan_ns);
        self.qos.min_separation_units = ring.ns_to_units(separation);
        self.qos.manual = manual;
        self.qos.lease_ns = lease_ns;
        if let LeanOwnership::Exclusive { strength } = qos.ownership {
            self.qos.exclusive = true;
            self.qos.strength = strength;
        }
        self.fast = false;
    }

    pub fn register_publisher(&mut self) -> Result<()> {
        if !self.announced {
            self.announced = true;
            self.publisher.announce();
        }
        if self.publisher.producer.is_none() {
            return Err(RenoirError::buffer_full("lean publisher table"));
        }
        if let Some(h) = self.publisher.producer {
            self.ring()
                .producer_entry(h.entry)
                .strength
                .store(self.qos.strength, Ordering::Release);
        }
        Ok(())
    }

    pub fn assert_liveliness(&mut self) -> Result<()> {
        self.register_publisher()?;
        if let Some(h) = self.publisher.producer {
            self.ring()
                .producer_entry(h.entry)
                .last_assert
                .store(monotonic_ns(), Ordering::Release);
        }
        Ok(())
    }

    pub fn liveliness(&mut self) -> LivelinessStatus {
        let (registered, alive) = self.ring().liveliness_counts(self.qos.lease_ns);
        let lost = alive == 0;
        if self.qos.was_alive && lost {
            self.qos.lost_count += 1;
        }
        self.qos.was_alive = !lost;
        LivelinessStatus {
            registered,
            alive,
            lost,
            lost_count: self.qos.lost_count,
        }
    }

    pub fn is_owner(&mut self) -> bool {
        let Some(mine) = self.publisher.producer.map(|h| h.entry) else {
            return false;
        };
        self.ring().current_owner(self.qos.lease_ns) == Some(mine)
    }

    pub(super) fn announce_slow(&mut self) -> Result<()> {
        if !self.announced {
            self.register_publisher()?;
        }
        if !self.qos.exclusive {
            self.fast = true;
            return Ok(());
        }
        let now = monotonic_ns();
        if now.saturating_sub(self.qos.owner_checked_ns) >= OWNER_RECHECK_NS {
            self.qos.owner_cached = self.is_owner();
            self.qos.owner_checked_ns = now;
        }
        if self.qos.owner_cached {
            Ok(())
        } else {
            Err(RenoirError::concurrency(
                "Another publisher with a higher strength owns this topic",
            ))
        }
    }

    pub fn stale_dropped(&self) -> u64 {
        self.qos.stale_dropped
    }

    pub fn filtered_out(&self) -> u64 {
        self.qos.filtered_out
    }

    pub fn try_read_stamped_qos(&mut self, out: &mut [u8]) -> Result<Option<u64>> {
        loop {
            let Some(stamp) = self.try_read_stamped(out)? else {
                return Ok(None);
            };
            let lifespan = self.qos.lifespan_units;
            if lifespan != 0 && self.ring().stamp_now().saturating_sub(stamp) > lifespan {
                self.qos.stale_dropped += 1;
                continue;
            }
            let separation = self.qos.min_separation_units;
            if separation != 0 {
                if let Some(last) = self.qos.last_delivered {
                    if stamp.saturating_sub(last) < separation {
                        self.qos.filtered_out += 1;
                        continue;
                    }
                }
            }
            self.qos.last_delivered = Some(stamp);
            return Ok(Some(stamp));
        }
    }

    pub fn lifespan(&self) -> Option<Duration> {
        (self.qos.lifespan_units != 0)
            .then(|| Duration::from_nanos(self.ring().ticks_to_ns(self.qos.lifespan_units)))
    }
}
