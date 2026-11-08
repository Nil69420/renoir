use std::{
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use super::{
    types::GateHandle, LeanCursor, LeanGatedCursor, LeanPublisher, LeanRead, LeanRing,
    LEAN_FIXED_HEADER_BYTES,
};
use crate::error::{RenoirError, Result};

pub const LEAN_MAX_CONSUMERS: usize = 32;
pub(super) const GATE_TABLE_BYTES: usize = LEAN_MAX_CONSUMERS * 64;
const FREE: u64 = 0;
pub(super) const ACTIVE: u64 = 1;
const EVICTED: u64 = 2;
const INIT: u64 = 3;
pub(super) const STATE_MASK: u64 = 3;
pub(super) const DEFAULT_GATE_TIMEOUT: Duration = Duration::from_millis(100);

#[repr(C)]
pub(super) struct GateEntry {
    pub(super) owner: AtomicU64,
    pub(super) pos: AtomicU64,
    pub(super) after: AtomicU64,
    pub(super) pid: AtomicU32,
    pad_pid: u32,
    pub(super) start: AtomicU64,
    pad: [u8; 24],
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

    pub fn set_gate_evicts_only_dead(&self, only_dead: bool) {
        self.gate_only_dead.store(only_dead, Ordering::Relaxed);
    }

    fn gate_holder_dead(&self, entry_index: usize) -> bool {
        let entry = self.gate_entry(entry_index);
        let pid = entry.pid.load(Ordering::Acquire);
        pid != 0 && !super::liveness::process_alive(pid, entry.start.load(Ordering::Acquire))
    }

    #[must_use]
    pub fn live_consumers(&self) -> usize {
        (0..LEAN_MAX_CONSUMERS)
            .filter(|&i| self.gate_entry_alive(i))
            .count()
    }

    #[must_use]
    pub fn live_consumers_before(&self, entry_index: usize) -> usize {
        (0..entry_index.min(LEAN_MAX_CONSUMERS))
            .filter(|&i| self.gate_entry_alive(i))
            .count()
    }

    fn gate_entry_alive(&self, entry_index: usize) -> bool {
        self.gate_entry(entry_index).owner.load(Ordering::Acquire) & STATE_MASK == ACTIVE
            && !self.gate_holder_dead(entry_index)
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
            upstream: None,
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
        if cursor.upstream.is_some() && self.upstream_limit(cursor) <= cursor.cursor.next {
            return Ok(LeanRead::Empty);
        }
        let read = self.try_read(&mut cursor.cursor, out);
        if matches!(read, Ok(LeanRead::Message)) {
            self.sync_gate(cursor);
        }
        read
    }

    pub fn skip_to_latest_gated(&self, cursor: &mut LeanGatedCursor, keep: u64) -> u64 {
        let skipped = if cursor.upstream.is_some() {
            let limit = self.upstream_limit(cursor);
            let target = self
                .published()
                .saturating_sub(keep.max(1) - 1)
                .max(1)
                .min(limit);
            if target > cursor.cursor.next {
                let skipped = target - cursor.cursor.next;
                cursor.cursor.lost += skipped;
                cursor.cursor.next = target;
                skipped
            } else {
                0
            }
        } else {
            self.skip_to_latest(&mut cursor.cursor, keep)
        };
        self.sync_gate(cursor);
        skipped
    }

    pub fn pending_gated(&self, cursor: &LeanGatedCursor) -> usize {
        let pending = self.pending(&cursor.cursor);
        match cursor.upstream {
            Some(up) => match self.upstream_pos(up) {
                Some(limit) => pending.min(limit.saturating_sub(cursor.cursor.next) as usize),
                None => pending,
            },
            None => pending,
        }
    }

    fn upstream_pos(&self, up: GateHandle) -> Option<u64> {
        let entry = self.gate_entry(up.entry);
        (entry.owner.load(Ordering::Acquire) == (up.token << 2) | ACTIVE)
            .then(|| entry.pos.load(Ordering::Acquire))
    }

    #[cold]
    pub(super) fn upstream_limit(&self, cursor: &mut LeanGatedCursor) -> u64 {
        let Some(up) = cursor.upstream else {
            return u64::MAX;
        };
        match self.upstream_pos(up) {
            Some(limit) => limit,
            None => {
                cursor.upstream = None;
                u64::MAX
            }
        }
    }

    pub fn set_upstream(&self, cursor: &mut LeanGatedCursor, upstream: usize) -> Result<()> {
        let Some(me) = cursor.handle else {
            return Err(RenoirError::invalid_parameter(
                "cursor",
                "Consumer is not registered",
            ));
        };
        if upstream >= LEAN_MAX_CONSUMERS || upstream == me.entry {
            return Err(RenoirError::invalid_parameter(
                "upstream",
                "Upstream must be another registered consumer",
            ));
        }
        let target = self.gate_entry(upstream);
        let owner = target.owner.load(Ordering::Acquire);
        if owner & STATE_MASK != ACTIVE {
            return Err(RenoirError::invalid_parameter(
                "upstream",
                "Upstream consumer is not registered",
            ));
        }
        let mut walk = target.after.load(Ordering::Acquire);
        for _ in 0..LEAN_MAX_CONSUMERS {
            if walk == 0 {
                break;
            }
            let next = ((walk & 0xFF) - 1) as usize;
            if next == me.entry {
                return Err(RenoirError::invalid_parameter(
                    "upstream",
                    "Upstream chain would form a cycle",
                ));
            }
            if next >= LEAN_MAX_CONSUMERS {
                break;
            }
            walk = self.gate_entry(next).after.load(Ordering::Acquire);
        }
        let token = owner >> 2;
        self.gate_entry(me.entry)
            .after
            .store((token << 8) | (upstream as u64 + 1), Ordering::Release);
        cursor.upstream = Some(GateHandle {
            entry: upstream,
            token,
        });
        Ok(())
    }

    fn claim_gate_entry(&self, start: u64) -> Result<GateHandle> {
        let pid = std::process::id();
        let process_start = super::liveness::process_start_time(pid);
        for pass in 0..2 {
            for entry_index in 0..LEAN_MAX_CONSUMERS {
                let entry = self.gate_entry(entry_index);
                let seen = entry.owner.load(Ordering::Acquire);
                let claimable = seen & STATE_MASK != ACTIVE && seen & STATE_MASK != INIT
                    || (seen & STATE_MASK == ACTIVE
                        && pass == 1
                        && self.gate_holder_dead(entry_index));
                if !claimable {
                    continue;
                }
                let token = (seen >> 2) + 1;
                if entry
                    .owner
                    .compare_exchange(
                        seen,
                        (token << 2) | INIT,
                        Ordering::AcqRel,
                        Ordering::Relaxed,
                    )
                    .is_err()
                {
                    continue;
                }
                entry.pos.store(start, Ordering::Relaxed);
                entry.after.store(0, Ordering::Relaxed);
                entry.pid.store(pid, Ordering::Relaxed);
                entry.start.store(process_start, Ordering::Relaxed);
                entry.owner.store((token << 2) | ACTIVE, Ordering::Release);
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
                Some((pos, since))
                    if pos == min
                        && since.elapsed() >= timeout
                        && (!self.ring.gate_only_dead.load(Ordering::Relaxed)
                            || self.ring.gate_holder_dead(laggard)) =>
                {
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
