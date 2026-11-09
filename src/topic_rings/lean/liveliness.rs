use std::{sync::atomic::Ordering, time::Duration};

use super::{
    gate::ACTIVE,
    liveness::{process_alive, LEAN_MAX_PRODUCERS, STATE_MASK},
    monotonic_ns, LeanChannel, LeanLivelinessKind, LeanOwnership, LeanPublisher, LeanQos, LeanRing,
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
    pub(super) lifespan_ns: u64,
    pub(super) min_separation_units: u64,
    pub(super) last_delivered: Option<u64>,
    pub(super) stale_dropped: u64,
    pub(super) filtered_out: u64,
    pub(super) manual: bool,
    pub(super) lease_ns: u64,
    pub(super) was_alive: bool,
    pub(super) lost_count: u64,
    pub(super) exclusive: bool,
    pub(super) assert_on_publish: bool,
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

pub(super) fn register(
    ring: &LeanRing,
    publisher: &mut LeanPublisher,
    state: &QosState,
) -> Result<()> {
    publisher.announce();
    let Some(h) = publisher.producer else {
        return Err(RenoirError::buffer_full("lean publisher table"));
    };
    ring.producer_entry(h.entry)
        .strength
        .store(state.strength, Ordering::Release);
    Ok(())
}

pub(super) fn assert_now(
    ring: &LeanRing,
    publisher: &mut LeanPublisher,
    state: &QosState,
) -> Result<()> {
    register(ring, publisher, state)?;
    if let Some(h) = publisher.producer {
        ring.producer_entry(h.entry)
            .last_assert
            .store(monotonic_ns(), Ordering::Release);
    }
    Ok(())
}

pub(super) fn status(ring: &LeanRing, state: &mut QosState) -> LivelinessStatus {
    let (registered, alive) = ring.liveliness_counts(state.lease_ns);
    let lost = alive == 0;
    if state.was_alive && lost {
        state.lost_count += 1;
    }
    state.was_alive = !lost;
    LivelinessStatus {
        registered,
        alive,
        lost,
        lost_count: state.lost_count,
    }
}

pub(super) fn is_owner(ring: &LeanRing, publisher: &LeanPublisher, state: &QosState) -> bool {
    let Some(mine) = publisher.producer.map(|h| h.entry) else {
        return false;
    };
    ring.current_owner(state.lease_ns) == Some(mine)
}

pub(super) fn wait_change(
    ring: &LeanRing,
    state: &mut QosState,
    timeout: Option<Duration>,
) -> Option<LivelinessStatus> {
    let start = std::time::Instant::now();
    let baseline = status(ring, state);
    let mut step = Duration::from_millis(1);
    loop {
        let waited = start.elapsed();
        let remaining = match timeout {
            Some(t) if waited >= t => return None,
            Some(t) => t - waited,
            None => step,
        };
        std::thread::sleep(step.min(remaining));
        let now = status(ring, state);
        if now.alive != baseline.alive || now.lost != baseline.lost {
            return Some(now);
        }
        step = (step * 2).min(Duration::from_millis(8));
    }
}

pub(super) fn gate(
    ring: &LeanRing,
    publisher: &mut LeanPublisher,
    state: &mut QosState,
) -> Result<bool> {
    if publisher.producer.is_none() {
        register(ring, publisher, state)?;
    }
    if state.assert_on_publish {
        if let Some(h) = publisher.producer {
            ring.producer_entry(h.entry)
                .last_assert
                .store(monotonic_ns(), Ordering::Release);
        }
    }
    if !state.exclusive {
        return Ok(!state.assert_on_publish);
    }
    let now = monotonic_ns();
    if now.saturating_sub(state.owner_checked_ns) >= OWNER_RECHECK_NS {
        state.owner_cached = is_owner(ring, publisher, state);
        state.owner_checked_ns = now;
    }
    if state.owner_cached {
        Ok(false)
    } else {
        Err(RenoirError::concurrency(
            "Another publisher with a higher strength owns this topic",
        ))
    }
}

pub(super) fn configure(state: &mut QosState, ring: &LeanRing, qos: &LeanQos, subscriber: bool) {
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
    let manual =
        qos.liveliness.kind == LeanLivelinessKind::Manual || (subscriber && ring.offered_manual());
    state.lease_ns = if manual {
        qos.liveliness.lease.map_or_else(
            || ring.offered_lease_ns(),
            |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
        )
    } else {
        0
    };
    state.lifespan_units = ring.ns_to_units(lifespan_ns);
    state.lifespan_ns = lifespan_ns;
    state.min_separation_units = ring.ns_to_units(separation);
    state.manual = manual;
    state.assert_on_publish = !subscriber && qos.liveliness.kind == LeanLivelinessKind::Manual;
    if let LeanOwnership::Exclusive { strength } = qos.ownership {
        state.exclusive = true;
        state.strength = strength;
    }
}

impl LeanChannel {
    pub(super) fn apply_qos(&mut self, qos: &LeanQos, subscriber: bool) {
        self.set_deadline(qos.deadline);
        let ring = self.ring().clone();
        configure(&mut self.qos, &ring, qos, subscriber);
        self.fast = false;
    }

    pub fn register_publisher(&mut self) -> Result<()> {
        self.announced = true;
        let ring = self.ring().clone();
        register(&ring, &mut self.publisher, &self.qos)
    }

    pub fn assert_liveliness(&mut self) -> Result<()> {
        self.announced = true;
        let ring = self.ring().clone();
        assert_now(&ring, &mut self.publisher, &self.qos)
    }

    pub fn liveliness(&mut self) -> LivelinessStatus {
        let ring = self.ring().clone();
        status(&ring, &mut self.qos)
    }

    pub fn wait_liveliness_change(
        &mut self,
        timeout: Option<Duration>,
    ) -> Option<LivelinessStatus> {
        let ring = self.ring().clone();
        wait_change(&ring, &mut self.qos, timeout)
    }

    pub fn is_owner(&mut self) -> bool {
        let ring = self.ring().clone();
        is_owner(&ring, &self.publisher, &self.qos)
    }

    pub(super) fn announce_slow(&mut self) -> Result<()> {
        self.announced = true;
        let ring = self.ring().clone();
        self.fast = gate(&ring, &mut self.publisher, &mut self.qos)?;
        Ok(())
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
