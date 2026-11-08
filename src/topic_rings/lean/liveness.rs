use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use super::gate::GATE_TABLE_BYTES;
use super::{LeanPublisher, LeanRing, LEAN_FIXED_HEADER_BYTES};

pub const LEAN_MAX_PRODUCERS: usize = 16;
pub(super) const PRODUCER_TABLE_BYTES: usize = LEAN_MAX_PRODUCERS * 64;
pub(super) const HARD_STALL: Duration = Duration::from_secs(10);
const FREE: u64 = 0;
const ACTIVE: u64 = 1;
const INIT: u64 = 2;
const STATE_MASK: u64 = 3;

#[repr(C)]
pub(super) struct ProducerEntry {
    owner: AtomicU64,
    pub(super) pid: AtomicU32,
    pad_pid: u32,
    start: AtomicU64,
    lo: AtomicU64,
    hi: AtomicU64,
    pad: [u8; 24],
}

const _: () = assert!(std::mem::size_of::<ProducerEntry>() == 64);
const _: () = assert!(std::mem::align_of::<ProducerEntry>() == 8);
const _: () = assert!((LEAN_FIXED_HEADER_BYTES + GATE_TABLE_BYTES).is_multiple_of(64));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Liveness {
    Alive,
    Dead,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProducerHandle {
    pub(super) entry: usize,
    token: u64,
}

fn process_start_time(pid: u32) -> u64 {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return 0;
    };
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return 0;
    };
    rest.split_whitespace()
        .nth(19)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn process_alive(pid: u32, start: u64) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
        return false;
    };
    if raw <= 0 {
        return false;
    }
    let rc = unsafe { libc::kill(raw, 0) };
    if rc != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
        return false;
    }
    start == 0 || process_start_time(pid) == start
}

impl LeanRing {
    #[inline(always)]
    pub(super) fn producer_entry(&self, i: usize) -> &ProducerEntry {
        unsafe {
            &*(self
                .header
                .as_ptr()
                .cast::<u8>()
                .add(LEAN_FIXED_HEADER_BYTES + GATE_TABLE_BYTES)
                .cast::<ProducerEntry>()
                .add(i))
        }
    }

    pub(super) fn register_producer(&self) -> Option<ProducerHandle> {
        let pid = std::process::id();
        let start = process_start_time(pid);
        for pass in 0..2 {
            for i in 0..LEAN_MAX_PRODUCERS {
                let entry = self.producer_entry(i);
                let seen = entry.owner.load(Ordering::Acquire);
                let reusable = match seen & STATE_MASK {
                    FREE => true,
                    ACTIVE => {
                        pass == 1
                            && !process_alive(
                                entry.pid.load(Ordering::Acquire),
                                entry.start.load(Ordering::Acquire),
                            )
                    }
                    _ => false,
                };
                if !reusable {
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
                entry.hi.store(0, Ordering::Relaxed);
                entry.lo.store(0, Ordering::Relaxed);
                entry.pid.store(pid, Ordering::Relaxed);
                entry.start.store(start, Ordering::Relaxed);
                entry.owner.store((token << 2) | ACTIVE, Ordering::Release);
                return Some(ProducerHandle { entry: i, token });
            }
        }
        None
    }

    pub(super) fn unregister_producer(&self, handle: ProducerHandle) {
        let entry = self.producer_entry(handle.entry);
        entry.hi.store(0, Ordering::Relaxed);
        let _ = entry.owner.compare_exchange(
            (handle.token << 2) | ACTIVE,
            (handle.token << 2) | FREE,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
    }

    #[must_use]
    pub fn live_publishers(&self) -> usize {
        (0..LEAN_MAX_PRODUCERS)
            .filter(|&i| {
                let entry = self.producer_entry(i);
                entry.owner.load(Ordering::Acquire) & STATE_MASK == ACTIVE
                    && process_alive(
                        entry.pid.load(Ordering::Acquire),
                        entry.start.load(Ordering::Acquire),
                    )
            })
            .count()
    }

    pub(super) fn owner_liveness(&self, idx: u64) -> Liveness {
        let mut found = Liveness::Unknown;
        for i in 0..LEAN_MAX_PRODUCERS {
            let entry = self.producer_entry(i);
            if entry.owner.load(Ordering::Acquire) & STATE_MASK != ACTIVE {
                continue;
            }
            let hi = entry.hi.load(Ordering::Acquire);
            let lo = entry.lo.load(Ordering::Acquire);
            if !(lo <= idx && idx < hi) {
                continue;
            }
            if process_alive(
                entry.pid.load(Ordering::Acquire),
                entry.start.load(Ordering::Acquire),
            ) {
                return Liveness::Alive;
            }
            found = Liveness::Dead;
        }
        found
    }
}

impl LeanPublisher {
    #[inline(always)]
    pub(super) fn begin_claim(&self) {
        if let Some(h) = self.producer {
            let entry = self.ring.producer_entry(h.entry);
            entry.lo.store(0, Ordering::Relaxed);
            entry.hi.store(u64::MAX, Ordering::Relaxed);
        }
    }

    #[inline(always)]
    pub(super) fn mark_claimed(&self, first: u64, count: u64) {
        if let Some(h) = self.producer {
            let entry = self.ring.producer_entry(h.entry);
            entry.hi.store(first + count, Ordering::Relaxed);
            entry.lo.store(first, Ordering::Relaxed);
        }
    }

    #[inline(always)]
    pub(super) fn end_claim(&self) {
        if let Some(h) = self.producer {
            self.ring
                .producer_entry(h.entry)
                .hi
                .store(0, Ordering::Relaxed);
        }
    }
}

impl Drop for LeanPublisher {
    fn drop(&mut self) {
        if let Some(h) = self.producer.take() {
            self.ring.unregister_producer(h);
        }
    }
}
