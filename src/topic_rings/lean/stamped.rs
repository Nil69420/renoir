use std::{
    ptr::NonNull,
    sync::atomic::{fence, AtomicU64, Ordering},
};

use crate::error::{RenoirError, Result};

use super::{
    atomic_load_bytes, atomic_store_bytes, clock, lean_slot_bytes, LeanClock, LeanCursor,
    LeanGatedCursor, LeanMode, LeanPublisher, LeanRead, LeanRing, LEAN_MAX_SLOT_LINES, LINE,
    SEQ_BYTES,
};

pub const STAMP_BYTES: usize = 8;
const SCRATCH_BYTES: usize = LEAN_MAX_SLOT_LINES * LINE;

fn stamped_geometry(data_size: usize) -> Result<(usize, usize)> {
    let msg_size = data_size
        .checked_add(STAMP_BYTES)
        .ok_or_else(|| RenoirError::invalid_parameter("msg_size", "Message size overflows"))?;
    let slot_bytes = lean_slot_bytes(msg_size).ok_or_else(|| {
        RenoirError::invalid_parameter(
            "msg_size",
            "Stamped lean message size must be between 1 and 496 bytes",
        )
    })?;
    Ok((msg_size, slot_bytes))
}

pub(super) fn tick_rate_for(clock_kind: LeanClock) -> Result<u64> {
    match clock_kind {
        LeanClock::Ticks => clock::tick_hz().ok_or_else(|| {
            RenoirError::platform("A stable cycle counter is not available; use nanosecond stamps")
        }),
        _ => Ok(0),
    }
}

impl LeanRing {
    /// # Safety
    ///
    /// Same contract as [`LeanRing::from_memory`]; `ring_bytes` is the slot area size computed
    /// from the stamped message size (`data_size` plus eight bytes).
    pub unsafe fn from_memory_stamped(
        memory: NonNull<u8>,
        memory_size: usize,
        ring_bytes: usize,
        data_size: usize,
        mode: LeanMode,
        clock_kind: LeanClock,
    ) -> Result<Self> {
        if !clock_kind.is_stamped() {
            return Err(RenoirError::invalid_parameter(
                "clock",
                "A stamped ring needs a nanosecond or tick clock",
            ));
        }
        let (msg_size, slot_bytes) = stamped_geometry(data_size)?;
        let slot_count = Self::slot_count_for(ring_bytes, msg_size);
        let tick_hz = tick_rate_for(clock_kind)?;
        Self::from_geometry_clock(
            memory,
            memory_size,
            slot_bytes,
            slot_count,
            msg_size,
            mode,
            clock_kind,
            tick_hz,
        )
    }

    #[must_use]
    pub fn data_size(&self) -> usize {
        if self.clock.is_stamped() {
            self.msg_size - STAMP_BYTES
        } else {
            self.msg_size
        }
    }

    #[inline]
    #[must_use]
    pub fn stamp_now(&self) -> u64 {
        if self.clock == LeanClock::Ticks {
            clock::ticks()
        } else {
            clock::monotonic_ns()
        }
    }

    #[must_use]
    pub fn ticks_to_ns(&self, ticks: u64) -> u64 {
        if self.clock == LeanClock::Ticks && self.tick_hz != 0 {
            u64::try_from(u128::from(ticks) * 1_000_000_000 / u128::from(self.tick_hz))
                .unwrap_or(u64::MAX)
        } else {
            ticks
        }
    }

    #[inline]
    pub fn try_read_stamped(&self, cursor: &mut LeanCursor, out: &mut [u8]) -> Result<Option<u64>> {
        let data_size = self.msg_size.wrapping_sub(STAMP_BYTES);
        if self.plain && self.clock.is_stamped() && out.len() >= data_size {
            let slot = self.slot_ptr(cursor.next);
            let seq = unsafe { &*(slot as *const AtomicU64) };
            let s1 = seq.load(Ordering::Acquire);
            let want = 2 * cursor.next;
            if s1 == want {
                let stamp =
                    unsafe { (*(slot.add(SEQ_BYTES) as *const AtomicU64)).load(Ordering::Relaxed) };
                unsafe {
                    atomic_load_bytes(
                        slot.add(SEQ_BYTES + STAMP_BYTES),
                        out.as_mut_ptr(),
                        data_size,
                    );
                }
                fence(Ordering::Acquire);
                if seq.load(Ordering::Relaxed) == s1 {
                    cursor.next += 1;
                    return Ok(Some(stamp));
                }
            } else if s1 < want {
                return Ok(None);
            }
        }
        self.read_stamped_through_scratch(out, |ring, scratch| ring.try_read(cursor, scratch))
    }

    pub fn try_read_stamped_gated(
        &self,
        cursor: &mut LeanGatedCursor,
        out: &mut [u8],
    ) -> Result<Option<u64>> {
        self.read_stamped_through_scratch(out, |ring, scratch| ring.try_read_gated(cursor, scratch))
    }

    #[inline(never)]
    fn read_stamped_through_scratch(
        &self,
        out: &mut [u8],
        read: impl FnOnce(&Self, &mut [u8]) -> Result<LeanRead>,
    ) -> Result<Option<u64>> {
        if !self.clock.is_stamped() {
            return Err(RenoirError::invalid_parameter(
                "ring",
                "The ring does not carry timestamps",
            ));
        }
        let data_size = self.msg_size - STAMP_BYTES;
        if out.len() < data_size {
            return Err(RenoirError::invalid_parameter(
                "buffer",
                "Output buffer smaller than the message size",
            ));
        }
        let mut scratch = [0u8; SCRATCH_BYTES];
        match read(self, &mut scratch[..self.msg_size])? {
            LeanRead::Empty => Ok(None),
            LeanRead::Message => {
                let mut stamp = [0u8; STAMP_BYTES];
                stamp.copy_from_slice(&scratch[..STAMP_BYTES]);
                out[..data_size].copy_from_slice(&scratch[STAMP_BYTES..self.msg_size]);
                Ok(Some(u64::from_ne_bytes(stamp)))
            }
        }
    }
}

impl LeanPublisher {
    #[inline]
    pub fn publish_at(&mut self, stamp: u64, data: &[u8]) -> Result<()> {
        let ring = &self.ring;
        if ring.plain && ring.clock.is_stamped() && data.len() + STAMP_BYTES == ring.msg_size {
            self.write_plain_stamped(stamp, data);
            return Ok(());
        }
        self.publish_stamped_general(stamp, data)
    }

    #[inline]
    pub fn publish_now(&mut self, data: &[u8]) -> Result<()> {
        let stamp = self.ring.stamp_now();
        self.publish_at(stamp, data)
    }

    #[inline(never)]
    fn publish_stamped_general(&mut self, stamp: u64, data: &[u8]) -> Result<()> {
        if !self.ring.clock.is_stamped() {
            return Err(RenoirError::invalid_parameter(
                "ring",
                "The ring does not carry timestamps",
            ));
        }
        if data.len() + STAMP_BYTES != self.ring.msg_size {
            return Err(RenoirError::invalid_parameter(
                "payload",
                "Lean payload must be exactly the topic message size",
            ));
        }
        let mut scratch = [0u8; SCRATCH_BYTES];
        scratch[..STAMP_BYTES].copy_from_slice(&stamp.to_ne_bytes());
        scratch[STAMP_BYTES..self.ring.msg_size].copy_from_slice(data);
        let msg_size = self.ring.msg_size;
        self.publish(&scratch[..msg_size])
    }

    #[inline(always)]
    fn write_plain_stamped(&mut self, stamp: u64, data: &[u8]) {
        let idx = self.next;
        let slot = self.ring.slot_ptr(idx);
        unsafe {
            let seq = &*(slot as *const AtomicU64);
            seq.store(2 * idx - 1, Ordering::Relaxed);
            fence(Ordering::Release);
            (*(slot.add(SEQ_BYTES) as *const AtomicU64)).store(stamp, Ordering::Relaxed);
            atomic_store_bytes(slot.add(SEQ_BYTES + STAMP_BYTES), data.as_ptr(), data.len());
            seq.store(2 * idx, Ordering::Release);
        }
        self.ring.write_idx().store(idx, Ordering::Release);
        self.next = idx + 1;
    }
}
