use std::{
    ptr::NonNull,
    sync::{
        atomic::{fence, AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc,
    },
};

use crate::error::{RenoirError, Result};

use super::atomic_bytes::{atomic_load_bytes, atomic_store_bytes};
use super::spsc::SESSION_MARKER_OFFSET;

mod batch;
mod channel;
mod clock;
mod cursor;
mod frame;
mod frame_channel;
#[cfg(test)]
mod frame_tests;
mod gate;
mod init;
mod inspect;
mod liveness;
#[cfg(test)]
mod liveness_tests;
mod multi;
#[cfg(kani)]
mod proofs;
mod qos;
mod stamped;
mod types;
mod wait;

pub use channel::LeanChannel;
pub use clock::{monotonic_ns, tick_hz, ticks, LeanClock};
pub use frame::{frame_slot_bytes, FrameLoan, FrameView, LEAN_MAX_FRAME_BYTES};
pub use frame_channel::FrameChannel;
pub use gate::LEAN_MAX_CONSUMERS;
pub use inspect::{inspect as inspect_lean_header, LeanInfo};
pub use liveness::LEAN_MAX_PRODUCERS;
pub use qos::{DeadlineWait, LeanQos, LeanReliability};
pub use stamped::STAMP_BYTES;
pub use types::{LeanCursor, LeanGatedCursor, LeanMode, LeanRead};

pub const LEAN_RING_MAGIC: u64 = 0x4C45_414E_5249_4E47;
const LEAN_FIXED_HEADER_BYTES: usize = 192;
pub const LEAN_HEADER_BYTES: usize =
    LEAN_FIXED_HEADER_BYTES + gate::GATE_TABLE_BYTES + liveness::PRODUCER_TABLE_BYTES;
pub const LEAN_MAX_SLOT_LINES: usize = 8;
const LINE: usize = 64;
const SEQ_BYTES: usize = 8;
const MAX_READ_ATTEMPTS: u32 = 64;

#[repr(C)]
#[derive(Debug)]
struct LeanHeader {
    magic: u64,
    slot_count: u64,
    session: AtomicU8,
    pad_session: [u8; 7],
    slot_bytes: u64,
    msg_size: u64,
    mode: u32,
    clock: u32,
    tick_hz: u64,
    pad_cold: [u8; 8],
    write_idx: AtomicU64,
    pad_write: [u8; 56],
    wake: AtomicU32,
    waiters: AtomicU32,
    claim: AtomicU64,
    pad_wake: [u8; 48],
}

const _: () = assert!(std::mem::offset_of!(LeanHeader, session) == SESSION_MARKER_OFFSET);
const _: () = assert!(std::mem::offset_of!(LeanHeader, write_idx) == 64);
const _: () = assert!(std::mem::offset_of!(LeanHeader, mode) == 40);
const _: () = assert!(std::mem::offset_of!(LeanHeader, clock) == 44);
const _: () = assert!(std::mem::offset_of!(LeanHeader, tick_hz) == 48);
const _: () = assert!(std::mem::offset_of!(LeanHeader, wake) == 128);
const _: () = assert!(std::mem::offset_of!(LeanHeader, claim) == 136);
const _: () = assert!(std::mem::size_of::<LeanHeader>() == LEAN_FIXED_HEADER_BYTES);

#[derive(Debug)]
pub struct LeanRing {
    header: NonNull<LeanHeader>,
    slots: NonNull<u8>,
    mask: u64,
    slot_bytes: usize,
    msg_size: usize,
    multi: bool,
    gated: bool,
    plain: bool,
    gate_timeout_ns: AtomicU64,
    gate_only_dead: AtomicBool,
    clock: LeanClock,
    tick_hz: u64,
}

unsafe impl Send for LeanRing {}
unsafe impl Sync for LeanRing {}

#[derive(Debug)]
pub struct LeanPublisher {
    ring: Arc<LeanRing>,
    next: u64,
    gate_cache: u64,
    blocked: Option<(u64, std::time::Instant)>,
    producer: Option<liveness::ProducerHandle>,
    announced: bool,
}

pub const fn lean_slot_bytes(msg_size: usize) -> Option<usize> {
    if msg_size == 0 {
        return None;
    }
    let Some(total) = msg_size.checked_add(SEQ_BYTES) else {
        return None;
    };
    let lines = total.div_ceil(LINE);
    if lines > LEAN_MAX_SLOT_LINES {
        None
    } else {
        Some(lines * LINE)
    }
}

impl LeanRing {
    pub fn slot_count_for(ring_bytes: usize, msg_size: usize) -> usize {
        let Some(slot_bytes) = lean_slot_bytes(msg_size) else {
            return 0;
        };
        let fit = ring_bytes / slot_bytes;
        if fit == 0 {
            0
        } else {
            1usize << (usize::BITS - 1 - fit.leading_zeros())
        }
    }

    pub fn required_memory_size(ring_bytes: usize, msg_size: usize) -> usize {
        LEAN_HEADER_BYTES
            + Self::slot_count_for(ring_bytes, msg_size) * lean_slot_bytes(msg_size).unwrap_or(0)
    }

    /// # Safety
    ///
    /// `memory` must be valid for reads and writes of `memory_size` bytes, 64-byte aligned
    /// for the latency guarantees (8-byte aligned for correctness), and outlive the ring.
    pub unsafe fn from_memory(
        memory: NonNull<u8>,
        memory_size: usize,
        ring_bytes: usize,
        msg_size: usize,
    ) -> Result<Self> {
        Self::from_memory_mode(
            memory,
            memory_size,
            ring_bytes,
            msg_size,
            LeanMode::SingleProducer,
        )
    }

    /// # Safety
    ///
    /// Same contract as [`LeanRing::from_memory`].
    pub unsafe fn from_memory_mode(
        memory: NonNull<u8>,
        memory_size: usize,
        ring_bytes: usize,
        msg_size: usize,
        mode: LeanMode,
    ) -> Result<Self> {
        let slot_bytes = lean_slot_bytes(msg_size).ok_or_else(|| {
            RenoirError::invalid_parameter(
                "max_payload_size",
                "Lean message size must be between 1 and 504 bytes",
            )
        })?;
        let slot_count = Self::slot_count_for(ring_bytes, msg_size);
        Self::from_geometry(memory, memory_size, slot_bytes, slot_count, msg_size, mode)
    }

    /// # Safety
    ///
    /// Same contract as [`LeanRing::from_memory`]; `slot_bytes` and `slot_count` must describe
    /// the layout the memory is (or will be) initialised with.
    pub(super) unsafe fn from_geometry(
        memory: NonNull<u8>,
        memory_size: usize,
        slot_bytes: usize,
        slot_count: usize,
        msg_size: usize,
        mode: LeanMode,
    ) -> Result<Self> {
        Self::from_geometry_clock(
            memory,
            memory_size,
            slot_bytes,
            slot_count,
            msg_size,
            mode,
            LeanClock::None,
            0,
        )
    }

    /// # Safety
    ///
    /// Same contract as [`LeanRing::from_geometry`].
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn from_geometry_clock(
        memory: NonNull<u8>,
        memory_size: usize,
        slot_bytes: usize,
        slot_count: usize,
        msg_size: usize,
        mode: LeanMode,
        clock: LeanClock,
        tick_hz: u64,
    ) -> Result<Self> {
        if slot_count < 2 {
            return Err(RenoirError::invalid_parameter(
                "ring_capacity",
                "Lean ring must hold at least 2 slots",
            ));
        }
        let required = LEAN_HEADER_BYTES + slot_count * slot_bytes;
        if memory_size < required {
            return Err(RenoirError::insufficient_space(required, memory_size));
        }
        if !(memory.as_ptr() as usize).is_multiple_of(std::mem::align_of::<LeanHeader>()) {
            return Err(RenoirError::alignment(
                memory.as_ptr() as usize,
                std::mem::align_of::<LeanHeader>(),
            ));
        }
        let header = memory.cast::<LeanHeader>();
        init::attach_header(
            header.as_ptr(),
            slot_count as u64,
            slot_bytes as u64,
            msg_size as u64,
            mode as u32,
            clock as u32,
            tick_hz,
        )?;
        let attached_clock = LeanClock::from_raw(header.as_ref().clock)
            .ok_or_else(|| RenoirError::invalid_parameter("ring", "Unknown timestamp mode"))?;
        let attached_hz = header.as_ref().tick_hz;
        Ok(Self {
            header,
            slots: NonNull::new_unchecked(memory.as_ptr().add(LEAN_HEADER_BYTES)),
            mask: slot_count as u64 - 1,
            slot_bytes,
            msg_size,
            multi: mode.is_multi_producer(),
            gated: mode.is_gated(),
            plain: !mode.is_multi_producer() && !mode.is_gated(),
            gate_timeout_ns: AtomicU64::new(gate::DEFAULT_GATE_TIMEOUT.as_nanos() as u64),
            gate_only_dead: AtomicBool::new(false),
            clock: attached_clock,
            tick_hz: attached_hz,
        })
    }

    pub fn msg_size(&self) -> usize {
        self.msg_size
    }

    #[must_use]
    pub fn clock(&self) -> LeanClock {
        self.clock
    }

    #[must_use]
    pub fn tick_hz(&self) -> u64 {
        self.tick_hz
    }

    pub fn slot_count(&self) -> usize {
        self.mask as usize + 1
    }

    pub fn published(&self) -> u64 {
        self.write_idx().load(Ordering::Acquire)
    }

    pub fn publisher(self: &Arc<Self>) -> LeanPublisher {
        LeanPublisher {
            ring: self.clone(),
            next: self.published() + 1,
            gate_cache: 0,
            blocked: None,
            producer: None,
            announced: false,
        }
    }

    #[inline]
    pub fn try_read(&self, cursor: &mut LeanCursor, out: &mut [u8]) -> Result<LeanRead> {
        if out.len() >= self.msg_size {
            let slot = self.slot_ptr(cursor.next);
            let seq = unsafe { &*(slot as *const AtomicU64) };
            let s1 = seq.load(Ordering::Acquire);
            let want = 2 * cursor.next;
            if s1 == want {
                unsafe { atomic_load_bytes(slot.add(SEQ_BYTES), out.as_mut_ptr(), self.msg_size) };
                fence(Ordering::Acquire);
                if seq.load(Ordering::Relaxed) == s1 {
                    cursor.next += 1;
                    return Ok(LeanRead::Message);
                }
            } else if s1 < want && !self.multi {
                return Ok(LeanRead::Empty);
            }
        }
        self.try_read_general(cursor, out)
    }

    #[inline(never)]
    fn try_read_general(&self, cursor: &mut LeanCursor, out: &mut [u8]) -> Result<LeanRead> {
        if out.len() < self.msg_size {
            return Err(RenoirError::invalid_parameter(
                "buffer",
                "Output buffer smaller than the lean message size",
            ));
        }
        let mut attempts = 0u32;
        loop {
            let slot = self.slot_ptr(cursor.next);
            let seq = unsafe { &*(slot as *const AtomicU64) };
            let s1 = seq.load(Ordering::Acquire);
            let want = 2 * cursor.next;
            if s1 == want {
                unsafe { atomic_load_bytes(slot.add(SEQ_BYTES), out.as_mut_ptr(), self.msg_size) };
                fence(Ordering::Acquire);
                if seq.load(Ordering::Relaxed) == s1 {
                    cursor.next += 1;
                    return Ok(LeanRead::Message);
                }
            } else if s1 < want {
                if self.multi && self.hole_expired(cursor) {
                    continue;
                }
                return Ok(LeanRead::Empty);
            }
            attempts += 1;
            if attempts > MAX_READ_ATTEMPTS {
                cursor.next += 1;
                cursor.lost += 1;
                return Err(RenoirError::serialization(
                    "Corrupt or unstable lean slot skipped",
                ));
            }
            self.resync(cursor);
        }
    }

    #[inline(always)]
    fn slot_ptr(&self, idx: u64) -> *mut u8 {
        let i = (idx & self.mask) as usize;
        unsafe { self.slots.as_ptr().add(i * self.slot_bytes) }
    }

    #[inline(always)]
    fn hdr(&self) -> &LeanHeader {
        unsafe { self.header.as_ref() }
    }

    #[inline(always)]
    fn write_idx(&self) -> &AtomicU64 {
        unsafe { &self.header.as_ref().write_idx }
    }
}

impl LeanPublisher {
    pub fn msg_size(&self) -> usize {
        self.ring.msg_size
    }

    #[inline]
    pub fn publish(&mut self, data: &[u8]) -> Result<()> {
        if self.ring.plain && data.len() == self.ring.msg_size {
            self.write_plain(data);
            return Ok(());
        }
        self.publish_general(data)
    }

    #[inline(never)]
    fn publish_general(&mut self, data: &[u8]) -> Result<()> {
        if data.len() != self.ring.msg_size {
            return Err(RenoirError::invalid_parameter(
                "payload",
                "Lean payload must be exactly the topic message size",
            ));
        }
        if self.ring.multi {
            return self.publish_multi(data);
        }
        if self.ring.gated {
            self.gate_admits(self.next)?;
        }
        self.write_plain(data);
        Ok(())
    }

    #[inline(always)]
    fn write_plain(&mut self, data: &[u8]) {
        let idx = self.next;
        let slot = self.ring.slot_ptr(idx);
        unsafe {
            let seq = &*(slot as *const AtomicU64);
            seq.store(2 * idx - 1, Ordering::Relaxed);
            fence(Ordering::Release);
            atomic_store_bytes(slot.add(SEQ_BYTES), data.as_ptr(), data.len());
            seq.store(2 * idx, Ordering::Release);
        }
        self.ring.write_idx().store(idx, Ordering::Release);
        self.next = idx + 1;
    }
}
