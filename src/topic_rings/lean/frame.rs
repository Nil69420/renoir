use std::sync::atomic::{fence, AtomicBool, AtomicU64, Ordering};

use super::{
    multi::{begin_slot_multi, commit_slot_multi},
    LeanGatedCursor, LeanMode, LeanPublisher, LeanRing, LINE, SEQ_BYTES,
};
use crate::error::{RenoirError, Result};
use std::ptr::NonNull;

pub const LEAN_MAX_FRAME_BYTES: usize = 64 << 20;
pub(super) const FRAME_HEADER_BYTES: usize = 8;
const TOMBSTONE: u32 = u32::MAX;

fn now_micros() -> u32 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64)
        .wrapping_mul(1_000_000)
        .wrapping_add(ts.tv_nsec as u64 / 1000) as u32
}

#[must_use]
pub const fn frame_slot_bytes(max_frame: usize) -> Option<usize> {
    if max_frame == 0 || max_frame > LEAN_MAX_FRAME_BYTES {
        None
    } else {
        Some((SEQ_BYTES + FRAME_HEADER_BYTES + max_frame).next_multiple_of(LINE))
    }
}

impl LeanRing {
    #[must_use]
    pub fn frame_slots_for(ring_bytes: usize, max_frame: usize) -> usize {
        let Some(slot_bytes) = frame_slot_bytes(max_frame) else {
            return 0;
        };
        let fit = ring_bytes / slot_bytes;
        if fit == 0 {
            0
        } else {
            1usize << (usize::BITS - 1 - fit.leading_zeros())
        }
    }

    #[must_use]
    pub fn required_frame_memory(ring_bytes: usize, max_frame: usize) -> usize {
        super::LEAN_HEADER_BYTES
            + Self::frame_slots_for(ring_bytes, max_frame)
                * frame_slot_bytes(max_frame).unwrap_or(0)
    }

    /// # Safety
    ///
    /// Same contract as [`LeanRing::from_memory`]. The memory must only ever be attached through
    /// this constructor: frame slots are written with plain stores and read in place, which is
    /// sound only because gated consumers hold their position while a view is alive.
    pub unsafe fn from_frame_memory(
        memory: NonNull<u8>,
        memory_size: usize,
        ring_bytes: usize,
        max_frame: usize,
        mode: LeanMode,
    ) -> Result<Self> {
        if !mode.is_gated() {
            return Err(RenoirError::invalid_parameter(
                "mode",
                "Frame rings must be gated so that views are owned",
            ));
        }
        let slot_bytes = frame_slot_bytes(max_frame).ok_or_else(|| {
            RenoirError::invalid_parameter("max_frame", "Frame size must be 1..=64 MiB")
        })?;
        let slot_count = Self::frame_slots_for(ring_bytes, max_frame);
        let mut ring = Self::from_geometry(
            memory,
            memory_size,
            slot_bytes,
            slot_count,
            FRAME_HEADER_BYTES + max_frame,
            mode,
        )?;
        ring.gate_only_dead = AtomicBool::new(true);
        Ok(ring)
    }

    #[must_use]
    pub fn max_frame(&self) -> usize {
        self.msg_size - FRAME_HEADER_BYTES
    }

    fn frame_data(&self, idx: u64) -> *mut u8 {
        unsafe { self.slot_ptr(idx).add(SEQ_BYTES + FRAME_HEADER_BYTES) }
    }

    fn frame_len_ptr(&self, idx: u64) -> *mut u32 {
        unsafe { self.slot_ptr(idx).add(SEQ_BYTES).cast::<u32>() }
    }

    fn frame_stamp_ptr(&self, idx: u64) -> *mut u32 {
        unsafe { self.slot_ptr(idx).add(SEQ_BYTES + 4).cast::<u32>() }
    }

    #[must_use]
    pub fn frame_age_micros(&self, idx: u64) -> u32 {
        now_micros().wrapping_sub(unsafe { self.frame_stamp_ptr(idx).read() })
    }

    pub fn peek_frame(&self, cursor: &mut LeanGatedCursor) -> Result<Option<(u64, usize)>> {
        loop {
            if cursor.upstream.is_some() && self.upstream_limit(cursor) <= cursor.cursor.next {
                return Ok(None);
            }
            let idx = cursor.cursor.next;
            let seq = unsafe { &*(self.slot_ptr(idx) as *const AtomicU64) };
            let seen = seq.load(Ordering::Acquire);
            let want = 2 * idx;
            if seen == want {
                let len = unsafe { self.frame_len_ptr(idx).read() };
                if len == TOMBSTONE {
                    cursor.cursor.next += 1;
                    self.sync_gate(cursor);
                    continue;
                }
                if len as usize > self.max_frame() {
                    cursor.cursor.next += 1;
                    cursor.cursor.lost += 1;
                    self.sync_gate(cursor);
                    return Err(RenoirError::serialization("Corrupt frame length skipped"));
                }
                return Ok(Some((idx, len as usize)));
            }
            if seen < want {
                if self.multi && self.hole_expired(&mut cursor.cursor) {
                    self.sync_gate(cursor);
                    continue;
                }
                return Ok(None);
            }
            self.resync(&mut cursor.cursor);
            self.sync_gate(cursor);
            return Err(RenoirError::concurrency(
                "Frame slot was overwritten before it could be viewed",
            ));
        }
    }

    pub fn release_frame(&self, cursor: &mut LeanGatedCursor, idx: u64) {
        cursor.cursor.next = idx + 1;
        self.sync_gate(cursor);
    }

    pub fn frame_ptr(&self, idx: u64) -> *const u8 {
        self.frame_data(idx)
    }

    pub fn frame_intact(&self, cursor: &LeanGatedCursor, idx: u64) -> bool {
        let still_registered = cursor.handle.is_some_and(|h| {
            let entry = self.gate_entry(h.entry);
            entry.owner.load(Ordering::Acquire) == (h.token << 2) | 1
        });
        let seq = unsafe { &*(self.slot_ptr(idx) as *const AtomicU64) };
        still_registered && seq.load(Ordering::Acquire) == 2 * idx
    }

    pub fn view_frame<'a>(
        &'a self,
        cursor: &'a mut LeanGatedCursor,
    ) -> Result<Option<FrameView<'a>>> {
        Ok(self.peek_frame(cursor)?.map(|(idx, len)| FrameView {
            ring: self,
            cursor,
            idx,
            len,
        }))
    }
}

pub struct FrameView<'a> {
    ring: &'a LeanRing,
    cursor: &'a mut LeanGatedCursor,
    idx: u64,
    len: usize,
}

impl<'a> FrameView<'a> {
    pub(super) fn new(
        ring: &'a LeanRing,
        cursor: &'a mut LeanGatedCursor,
        idx: u64,
        len: usize,
    ) -> Self {
        Self {
            ring,
            cursor,
            idx,
            len,
        }
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ring.frame_data(self.idx), self.len) }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn index(&self) -> u64 {
        self.idx
    }

    #[must_use]
    pub fn age(&self) -> std::time::Duration {
        std::time::Duration::from_micros(u64::from(self.ring.frame_age_micros(self.idx)))
    }

    #[must_use]
    pub fn is_intact(&self) -> bool {
        self.ring.frame_intact(self.cursor, self.idx)
    }

    pub fn release(self) {
        drop(self);
    }
}

impl Drop for FrameView<'_> {
    fn drop(&mut self) {
        self.ring.release_frame(self.cursor, self.idx);
    }
}

pub struct FrameLoan<'a> {
    publisher: &'a mut LeanPublisher,
    idx: u64,
    len: usize,
    done: bool,
}

impl LeanPublisher {
    pub fn begin_frame(&mut self, len: usize) -> Result<u64> {
        if len == 0 || len > self.ring.max_frame() {
            return Err(RenoirError::invalid_parameter(
                "len",
                "Frame length must be 1..=max_frame",
            ));
        }
        if !self.ring.gated {
            return Err(RenoirError::invalid_parameter(
                "ring",
                "Frames need a gated ring",
            ));
        }
        if self.ring.multi {
            self.announce();
            let idx = self.claim_index()?;
            if let Err(e) = begin_slot_multi(&self.ring, idx) {
                self.end_claim();
                return Err(e);
            }
            Ok(idx)
        } else {
            self.gate_admits(self.next)?;
            let idx = self.next;
            let seq = unsafe { &*(self.ring.slot_ptr(idx) as *const AtomicU64) };
            seq.store(2 * idx - 1, Ordering::Relaxed);
            fence(Ordering::Release);
            Ok(idx)
        }
    }

    pub fn frame_buffer(&self, idx: u64) -> *mut u8 {
        self.ring.frame_data(idx)
    }

    pub fn end_frame(&mut self, idx: u64, len: Option<usize>) -> Result<()> {
        let ring = self.ring.clone();
        let stored = len.map_or(TOMBSTONE, |l| l as u32);
        if ring.multi || len.is_some() {
            unsafe {
                ring.frame_len_ptr(idx).write(stored);
                ring.frame_stamp_ptr(idx).write(now_micros());
            }
        }
        if ring.multi {
            let committed = commit_slot_multi(&ring, idx);
            self.end_claim();
            committed?;
            ring.write_idx().fetch_max(idx, Ordering::Release);
        } else if len.is_none() {
            let seq = unsafe { &*(ring.slot_ptr(idx) as *const AtomicU64) };
            let previous = idx.saturating_sub(ring.mask + 1) * 2;
            seq.store(previous, Ordering::Release);
        } else {
            let seq = unsafe { &*(ring.slot_ptr(idx) as *const AtomicU64) };
            seq.store(2 * idx, Ordering::Release);
            ring.write_idx().store(idx, Ordering::Release);
            self.next = idx + 1;
        }
        Ok(())
    }

    pub fn loan_frame(&mut self, len: usize) -> Result<FrameLoan<'_>> {
        let idx = self.begin_frame(len)?;
        Ok(FrameLoan {
            publisher: self,
            idx,
            len,
            done: false,
        })
    }
}

impl FrameLoan<'_> {
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.publisher.frame_buffer(self.idx), self.len) }
    }

    pub fn commit(mut self) -> Result<()> {
        self.done = true;
        self.publisher.end_frame(self.idx, Some(self.len))
    }

    pub fn commit_notify(mut self) -> Result<()> {
        self.done = true;
        let result = self.publisher.end_frame(self.idx, Some(self.len));
        if result.is_ok() {
            self.publisher.ring.notify();
        }
        result
    }

    pub fn abort(mut self) {
        self.done = true;
        let _ = self.publisher.end_frame(self.idx, None);
    }
}

impl Drop for FrameLoan<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.publisher.end_frame(self.idx, None);
        }
    }
}
