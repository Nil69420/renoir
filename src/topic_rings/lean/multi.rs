use std::{
    sync::atomic::{fence, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use super::{
    atomic_store_bytes,
    liveness::{Liveness, HARD_STALL},
    LeanCursor, LeanPublisher, LeanRing, SEQ_BYTES,
};
use crate::error::{RenoirError, Result};

const HOLE_TIMEOUT: Duration = Duration::from_millis(5);
const TAKEOVER_TIMEOUT: Duration = Duration::from_millis(50);
const POLLS_PER_HOLE_CHECK: u32 = 64;
const SPINS_PER_CLOCK_CHECK: u32 = 2048;

impl LeanRing {
    #[cold]
    pub(super) fn hole_expired(&self, cursor: &mut LeanCursor) -> bool {
        cursor.empty_polls = cursor.empty_polls.wrapping_add(1);
        if !cursor.empty_polls.is_multiple_of(POLLS_PER_HOLE_CHECK) {
            return false;
        }
        if self.hdr().claim.load(Ordering::Acquire) < cursor.next {
            cursor.stalled = None;
            return false;
        }
        let limit = if self.gated {
            Duration::from_nanos(self.gate_timeout_ns.load(Ordering::Relaxed)).max(HOLE_TIMEOUT)
        } else {
            HOLE_TIMEOUT
        };
        match cursor.stalled {
            Some((idx, since)) if idx == cursor.next => {
                let waited = since.elapsed();
                if waited < HOLE_TIMEOUT {
                    return false;
                }
                let skip = match self.owner_liveness(idx) {
                    Liveness::Alive => waited >= HARD_STALL,
                    Liveness::Dead => true,
                    Liveness::Unknown => waited >= limit,
                };
                if !skip {
                    return false;
                }
                cursor.next += 1;
                cursor.lost += 1;
                cursor.stalled = None;
                true
            }
            _ => {
                cursor.stalled = Some((cursor.next, Instant::now()));
                false
            }
        }
    }
}

impl LeanPublisher {
    pub(super) fn claim_index(&mut self) -> Result<u64> {
        if !self.ring.gated {
            self.begin_claim();
            let idx = self.ring.hdr().claim.fetch_add(1, Ordering::AcqRel) + 1;
            self.mark_claimed(idx, 1);
            return Ok(idx);
        }
        loop {
            let current = self.ring.hdr().claim.load(Ordering::Acquire);
            let admitted = current + 1;
            if let Err(e) = self.gate_admits(admitted) {
                self.end_claim();
                return Err(e);
            }
            self.begin_claim();
            if self
                .ring
                .hdr()
                .claim
                .compare_exchange_weak(current, admitted, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                self.mark_claimed(admitted, 1);
                return Ok(admitted);
            }
        }
    }

    pub(super) fn publish_multi(&mut self, data: &[u8]) -> Result<()> {
        let idx = self.claim_index()?;
        let filled = fill_slot_multi(&self.ring, idx, data);
        self.end_claim();
        filled?;
        self.ring.write_idx().fetch_max(idx, Ordering::Release);
        Ok(())
    }

    pub(super) fn claim_range(&mut self, count: u64) -> u64 {
        self.begin_claim();
        let first = self.ring.hdr().claim.fetch_add(count, Ordering::AcqRel) + 1;
        self.mark_claimed(first, count);
        first
    }
}

pub(super) fn fill_slot_multi(ring: &LeanRing, idx: u64, data: &[u8]) -> Result<()> {
    let slot = ring.slot_ptr(idx);
    let seq = unsafe { &*(slot as *const AtomicU64) };
    let previous = idx.saturating_sub(ring.mask + 1) * 2;
    let writing = 2 * idx - 1;
    acquire_slot(ring, seq, previous, writing)?;
    fence(Ordering::Release);
    unsafe { atomic_store_bytes(slot.add(SEQ_BYTES), data.as_ptr(), data.len()) };
    if seq
        .compare_exchange(writing, 2 * idx, Ordering::Release, Ordering::Relaxed)
        .is_err()
    {
        return Err(RenoirError::concurrency(
            "Lean slot was taken over by another producer",
        ));
    }
    Ok(())
}

fn acquire_slot(ring: &LeanRing, seq: &AtomicU64, previous: u64, writing: u64) -> Result<()> {
    let mut spins = 0u32;
    let mut started: Option<Instant> = None;
    loop {
        match seq.compare_exchange_weak(previous, writing, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return Ok(()),
            Err(seen) => {
                if seen >= writing {
                    return Err(RenoirError::concurrency(
                        "Lean slot was lapped before the producer could write",
                    ));
                }
                spins += 1;
                if spins.is_multiple_of(SPINS_PER_CLOCK_CHECK) {
                    let waited = started.get_or_insert_with(Instant::now).elapsed();
                    if waited >= TAKEOVER_TIMEOUT
                        && seen != previous
                        && may_take_over(ring, seen, previous, waited)
                        && seq
                            .compare_exchange(seen, writing, Ordering::AcqRel, Ordering::Relaxed)
                            .is_ok()
                    {
                        return Ok(());
                    }
                    std::thread::yield_now();
                }
                std::hint::spin_loop();
            }
        }
    }
}

fn may_take_over(ring: &LeanRing, seen: u64, previous: u64, waited: Duration) -> bool {
    let stuck = if seen & 1 == 1 {
        seen.div_ceil(2)
    } else {
        previous / 2
    };
    match ring.owner_liveness(stuck) {
        Liveness::Alive => waited >= HARD_STALL,
        Liveness::Dead | Liveness::Unknown => true,
    }
}
