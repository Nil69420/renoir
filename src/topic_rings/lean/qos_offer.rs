use std::{sync::atomic::Ordering, time::Duration};

use super::{LeanLivelinessKind, LeanOwnership, LeanQos, LeanRing};
use crate::error::{RenoirError, Result};

const VALID: u64 = 1;
const MANUAL: u64 = 2;
const EXCLUSIVE: u64 = 4;
const BUSY: u64 = 8;
const SPINS: u32 = 4000;
const TAKEOVER_AFTER: u32 = 3000;

fn nanos(duration: Option<Duration>) -> u64 {
    duration.map_or(0, |d| {
        u64::try_from(d.as_nanos()).unwrap_or(u64::MAX).max(1)
    })
}

fn encode(qos: &LeanQos) -> u64 {
    let mut word = VALID;
    if qos.liveliness.kind == LeanLivelinessKind::Manual {
        word |= MANUAL;
    }
    if matches!(qos.ownership, LeanOwnership::Exclusive { .. }) {
        word |= EXCLUSIVE;
    }
    word
}

fn incompatible(why: &'static str) -> RenoirError {
    RenoirError::invalid_parameter("qos", why)
}

impl LeanRing {
    pub(super) fn offer(&self, qos: &LeanQos) -> Result<()> {
        let h = self.hdr();
        let word = encode(qos);
        let deadline = nanos(qos.deadline);
        let lifespan = nanos(qos.lifespan);
        let lease = nanos(qos.liveliness.lease);
        let publish = || {
            h.offered_deadline_ns.store(deadline, Ordering::Relaxed);
            h.offered_lifespan_ns.store(lifespan, Ordering::Relaxed);
            h.offered_lease_ns.store(lease, Ordering::Relaxed);
            h.qos_offer.store(word, Ordering::Release);
        };
        for attempt in 0..SPINS {
            let seen = h.qos_offer.load(Ordering::Acquire);
            if seen == 0 {
                if h.qos_offer
                    .compare_exchange(0, BUSY, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
                {
                    publish();
                    return Ok(());
                }
                continue;
            }
            if seen & BUSY != 0 {
                if attempt >= TAKEOVER_AFTER {
                    publish();
                    return Ok(());
                }
                std::thread::yield_now();
                continue;
            }
            let same = seen == word
                && h.offered_deadline_ns.load(Ordering::Relaxed) == deadline
                && h.offered_lifespan_ns.load(Ordering::Relaxed) == lifespan
                && h.offered_lease_ns.load(Ordering::Relaxed) == lease;
            return if same {
                Ok(())
            } else {
                Err(incompatible("The ring already offers a different QoS"))
            };
        }
        Err(incompatible("The ring QoS offer never settled"))
    }

    fn settled_offer(&self) -> Option<u64> {
        let h = self.hdr();
        for _ in 0..SPINS {
            let seen = h.qos_offer.load(Ordering::Acquire);
            if seen & BUSY == 0 {
                return (seen & VALID != 0).then_some(seen);
            }
            std::thread::yield_now();
        }
        None
    }

    pub(super) fn check_offer(&self, qos: &LeanQos) -> Result<()> {
        let Some(word) = self.settled_offer() else {
            return Ok(());
        };
        let h = self.hdr();
        let offered_deadline = h.offered_deadline_ns.load(Ordering::Relaxed);
        if let Some(requested) = qos.deadline {
            if offered_deadline == 0 || offered_deadline > nanos(Some(requested)) {
                return Err(incompatible(
                    "The publisher offers a longer deadline than requested",
                ));
            }
        }
        if qos.liveliness.kind == LeanLivelinessKind::Manual {
            let lease = h.offered_lease_ns.load(Ordering::Relaxed);
            if word & MANUAL == 0 || lease == 0 || lease > nanos(qos.liveliness.lease) {
                return Err(incompatible(
                    "The publisher does not offer manual liveliness within the requested lease",
                ));
            }
        }
        let wants_exclusive = matches!(qos.ownership, LeanOwnership::Exclusive { .. });
        if wants_exclusive != (word & EXCLUSIVE != 0) {
            return Err(incompatible("Ownership kinds differ"));
        }
        Ok(())
    }

    pub(super) fn offered_lifespan_ns(&self) -> u64 {
        self.hdr().offered_lifespan_ns.load(Ordering::Relaxed)
    }

    pub(super) fn offered_lease_ns(&self) -> u64 {
        self.hdr().offered_lease_ns.load(Ordering::Relaxed)
    }

    pub(super) fn offered_manual(&self) -> bool {
        self.settled_offer().is_some_and(|w| w & MANUAL != 0)
    }

    pub(super) fn ns_to_units(&self, ns: u64) -> u64 {
        if self.tick_hz() == 0
            || !self.clock().is_stamped()
            || self.clock() == super::LeanClock::Nanoseconds
        {
            return ns;
        }
        u64::try_from(u128::from(ns) * u128::from(self.tick_hz()) / 1_000_000_000)
            .unwrap_or(u64::MAX)
    }
}
