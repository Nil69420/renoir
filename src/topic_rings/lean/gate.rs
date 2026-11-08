use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use super::{
    types::GateHandle, LeanCursor, LeanGatedCursor, LeanPublisher, LeanRead, LeanRing,
    LEAN_FIXED_HEADER_BYTES,
};
use crate::error::{RenoirError, Result};

pub const LEAN_MAX_CONSUMERS: usize = 8;
pub(super) const GATE_TABLE_BYTES: usize = LEAN_MAX_CONSUMERS * 64;
const FREE: u64 = 0;
pub(super) const ACTIVE: u64 = 1;
const EVICTED: u64 = 2;
pub(super) const STATE_MASK: u64 = 3;
pub(super) const DEFAULT_GATE_TIMEOUT: Duration = Duration::from_millis(100);

#[repr(C)]
pub(super) struct GateEntry {
    pub(super) owner: AtomicU64,
    pub(super) pos: AtomicU64,
    pad: [u8; 48],
}

const _: () = assert!(std::mem::size_of::<GateEntry>() == 64);
const _: () = assert!(std::mem::align_of::<GateEntry>() == 8);

impl LeanRing {
    #[inline(always)]
    pub(super) fn gate_entry(&self, i: usize) -> &GateEntry {
        unsafe {
            &*(self
                .header
                .as_ptr()
                .cast::<u8>()
                .add(LEAN_FIXED_HEADER_BYTES)
                .cast::<GateEntry>()
                .add(i))
        }
    }

    pub fn set_gate_timeout(&self, timeout: Duration) {
        self.gate_timeout_ns.store(
            u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    pub fn register_consumer(&self, from_oldest: bool) -> Result<LeanGatedCursor> {
        if !self.gated {
            return Err(RenoirError::invalid_parameter(
                "ring",
                "Consumer registration needs a gated lean ring",
            ));
        }
        let published = self.published();
        let start = if from_oldest {
            published.saturating_sub(self.mask).max(1)
        } else {
            published + 1
        };
        Ok(LeanGatedCursor {
            cursor: LeanCursor::at(start),
            handle: Some(self.claim_gate_entry(start)?),
        })
    }

    pub fn unregister_consumer(&self, cursor: &mut LeanGatedCursor) {
        if let Some(g) = cursor.handle.take() {
            let entry = self.gate_entry(g.entry);
            let _ = entry.owner.compare_exchange(
                (g.token << 2) | ACTIVE,
                (g.token << 2) | FREE,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
        }
    }

    #[inline]
    pub fn try_read_gated(&self, cursor: &mut LeanGatedCursor, out: &mut [u8]) -> Result<LeanRead> {
        let read = self.try_read(&mut cursor.cursor, out);
        if matches!(read, Ok(LeanRead::Message)) {
            self.sync_gate(cursor);
        }
        read
    }

    pub fn skip_to_latest_gated(&self, cursor: &mut LeanGatedCursor, keep: u64) -> u64 {
        let skipped = self.skip_to_latest(&mut cursor.cursor, keep);
        self.sync_gate(cursor);
        skipped
    }

    pub fn pending_gated(&self, cursor: &LeanGatedCursor) -> usize {
        self.pending(&cursor.cursor)
    }

    fn claim_gate_entry(&self, start: u64) -> Result<GateHandle> {
        for entry_index in 0..LEAN_MAX_CONSUMERS {
            let entry = self.gate_entry(entry_index);
            let seen = entry.owner.load(Ordering::Acquire);
            if seen & STATE_MASK == ACTIVE {
                continue;
            }
            let token = (seen >> 2) + 1;
            entry.pos.store(start, Ordering::Relaxed);
            if entry
                .owner
                .compare_exchange(
                    seen,
                    (token << 2) | ACTIVE,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return Ok(GateHandle {
                    entry: entry_index,
                    token,
                });
            }
        }
        Err(RenoirError::concurrency(
            "All lean consumer slots are in use",
        ))
    }

    #[inline]
    pub(super) fn sync_gate(&self, cursor: &mut LeanGatedCursor) {
        let Some(handle) = cursor.handle else {
            return;
        };
        let entry = self.gate_entry(handle.entry);
        if entry.owner.load(Ordering::Relaxed) == (handle.token << 2) | ACTIVE {
            entry.pos.store(cursor.cursor.next, Ordering::Release);
        } else {
            self.gate_reclaim(cursor);
        }
    }

    #[cold]
    fn gate_reclaim(&self, cursor: &mut LeanGatedCursor) {
        cursor.handle = self.claim_gate_entry(cursor.cursor.next).ok();
    }

    fn gate_minimum(&self) -> (u64, usize) {
        let mut min = u64::MAX;
        let mut laggard = 0;
        for i in 0..LEAN_MAX_CONSUMERS {
            let entry = self.gate_entry(i);
            if entry.owner.load(Ordering::Acquire) & STATE_MASK != ACTIVE {
                continue;
            }
            let pos = entry.pos.load(Ordering::Acquire);
            if pos < min {
                min = pos;
                laggard = i;
            }
        }
        (min, laggard)
    }

    fn evict(&self, entry_index: usize) {
        let entry = self.gate_entry(entry_index);
        let seen = entry.owner.load(Ordering::Acquire);
        if seen & STATE_MASK == ACTIVE {
            let _ = entry.owner.compare_exchange(
                seen,
                (seen & !STATE_MASK) | EVICTED,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
        }
    }
}

impl LeanPublisher {
    #[inline(always)]
    pub(super) fn gate_admits(&mut self, idx: u64) -> Result<()> {
        if idx < self.gate_cache.saturating_add(self.ring.mask + 1) {
            return Ok(());
        }
        self.gate_slow(idx)
    }

    #[cold]
    fn gate_slow(&mut self, idx: u64) -> Result<()> {
        let capacity = self.ring.mask + 1;
        loop {
            let (min, laggard) = self.ring.gate_minimum();
            self.gate_cache = min;
            if idx < min.saturating_add(capacity) {
                self.blocked = None;
                return Ok(());
            }
            let timeout = Duration::from_nanos(self.ring.gate_timeout_ns.load(Ordering::Relaxed));
            match self.blocked {
                Some((pos, since)) if pos == min && since.elapsed() >= timeout => {
                    self.ring.evict(laggard);
                    self.blocked = None;
                }
                Some((pos, _)) if pos == min => return Err(RenoirError::buffer_full("lean ring")),
                _ => {
                    self.blocked = Some((min, Instant::now()));
                    return Err(RenoirError::buffer_full("lean ring"));
                }
            }
        }
    }
}
