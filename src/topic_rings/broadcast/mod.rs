use std::{
    ptr::NonNull,
    sync::{
        atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
};

use crate::{
    error::{RenoirError, Result},
    topic::{Message, MessageHeader, MessagePayload, TopicStats},
};

use crate::topic_rings::atomic_bytes::{atomic_load_bytes, atomic_store_bytes};
use crate::topic_rings::spsc::SESSION_MARKER_OFFSET;

pub const BROADCAST_RING_MAGIC: u64 = 0x4243_4153_5452_4E47;

const HEADER_BYTES: usize = 64;
const SLOT_HEADER_BYTES: usize = 16;
const SLOT_ALIGN: usize = 64;
const MESSAGE_AREA_BYTES: usize = 112;
const PAYLOAD_OFFSET: usize = SLOT_HEADER_BYTES + MESSAGE_AREA_BYTES;

#[repr(C)]
#[derive(Debug)]
struct BroadcastHeader {
    magic: u64,
    slot_count: usize,
    session: u8,
    stride: usize,
    payload_cap: usize,
    write_idx: AtomicU64,
    wake: AtomicU32,
    waiters: AtomicU32,
}

const _: () = assert!(std::mem::offset_of!(BroadcastHeader, session) == SESSION_MARKER_OFFSET);
const _: () = assert!(std::mem::size_of::<BroadcastHeader>() <= HEADER_BYTES);

#[repr(C)]
struct SlotHeader {
    seq: AtomicU64,
    len: AtomicU32,
    reserved: u32,
}

const _: () = assert!(MessageHeader::SIZE <= MESSAGE_AREA_BYTES);
const _: () = assert!(PAYLOAD_OFFSET.is_multiple_of(64));
const _: () = assert!(HEADER_BYTES.is_multiple_of(64));

const _: () = assert!(std::mem::size_of::<SlotHeader>() == SLOT_HEADER_BYTES);

mod init;
mod types;
mod wait;
mod zero_copy;

pub use types::{BroadcastCursor, BroadcastReservation, ReadInto};

#[derive(Debug)]
pub struct BroadcastTopicRing {
    header: NonNull<BroadcastHeader>,
    slots: NonNull<u8>,
    slot_count: u64,
    stride: usize,
    payload_cap: usize,
    stats: Arc<TopicStats>,
    reserved: AtomicBool,
}

unsafe impl Send for BroadcastTopicRing {}
unsafe impl Sync for BroadcastTopicRing {}

fn stride_for(payload_cap: usize) -> usize {
    (PAYLOAD_OFFSET + payload_cap.next_multiple_of(8)).div_ceil(SLOT_ALIGN) * SLOT_ALIGN
}

impl BroadcastTopicRing {
    pub fn slot_stride(payload_cap: usize) -> usize {
        stride_for(payload_cap)
    }

    pub fn slot_count_for(ring_bytes: usize, payload_cap: usize) -> usize {
        ring_bytes / stride_for(payload_cap)
    }

    pub fn required_memory_size(ring_bytes: usize, payload_cap: usize) -> usize {
        HEADER_BYTES + Self::slot_count_for(ring_bytes, payload_cap) * stride_for(payload_cap)
    }

    /// # Safety
    ///
    /// `memory` must be valid for reads and writes of `memory_size` bytes, at least
    /// 8-byte aligned, and must outlive the returned ring.
    pub unsafe fn from_memory(
        memory: NonNull<u8>,
        memory_size: usize,
        ring_bytes: usize,
        payload_cap: usize,
        stats: Arc<TopicStats>,
    ) -> Result<Self> {
        if payload_cap == 0 || payload_cap > u32::MAX as usize {
            return Err(RenoirError::invalid_parameter(
                "max_payload_size",
                "Broadcast payload capacity must be between 1 and u32::MAX",
            ));
        }
        let stride = stride_for(payload_cap);
        let slot_count = Self::slot_count_for(ring_bytes, payload_cap);
        if slot_count < 2 {
            return Err(RenoirError::invalid_parameter(
                "ring_capacity",
                "Broadcast ring must hold at least 2 slots of max_payload_size",
            ));
        }
        let required = HEADER_BYTES + slot_count * stride;
        if memory_size < required {
            return Err(RenoirError::insufficient_space(required, memory_size));
        }
        if !(memory.as_ptr() as usize).is_multiple_of(std::mem::align_of::<BroadcastHeader>()) {
            return Err(RenoirError::alignment(
                memory.as_ptr() as usize,
                std::mem::align_of::<BroadcastHeader>(),
            ));
        }

        let header_ptr = memory.cast::<BroadcastHeader>();
        init::attach_header(header_ptr.as_ptr(), slot_count, stride, payload_cap)?;

        Ok(Self {
            header: header_ptr,
            slots: unsafe { NonNull::new_unchecked(memory.as_ptr().add(HEADER_BYTES)) },
            slot_count: slot_count as u64,
            stride,
            payload_cap,
            stats,
            reserved: AtomicBool::new(false),
        })
    }

    pub fn slot_count(&self) -> usize {
        self.slot_count as usize
    }

    pub fn payload_capacity(&self) -> usize {
        self.payload_cap
    }

    pub fn published_count(&self) -> u64 {
        self.write_idx().load(Ordering::Acquire)
    }

    pub fn cursor_latest(&self) -> BroadcastCursor {
        BroadcastCursor {
            next: self.published_count() + 1,
            overruns: 0,
            skipped: 0,
        }
    }

    pub fn cursor_oldest(&self) -> BroadcastCursor {
        let w = self.published_count();
        BroadcastCursor {
            next: w.saturating_sub(self.slot_count - 1).max(1),
            overruns: 0,
            skipped: 0,
        }
    }

    pub fn try_publish(&self, message: &Message) -> Result<()> {
        let data = match &message.payload {
            MessagePayload::Inline(data) => data,
            MessagePayload::Descriptor(_) => {
                return Err(RenoirError::serialization(
                    "Broadcast topics carry inline payloads only",
                ))
            }
        };
        if data.len() > self.payload_cap {
            return Err(RenoirError::invalid_parameter(
                "payload",
                "Payload exceeds broadcast slot capacity",
            ));
        }

        let idx = self.write_idx().load(Ordering::Relaxed) + 1;
        let slot = self.slot_ptr(idx);
        unsafe {
            let sh = slot as *mut SlotHeader;
            (*sh).seq.store(2 * idx - 1, Ordering::Relaxed);
            fence(Ordering::Release);
            (*sh).len.store(data.len() as u32, Ordering::Relaxed);
            atomic_store_bytes(
                slot.add(SLOT_HEADER_BYTES),
                &message.header as *const MessageHeader as *const u8,
                MessageHeader::SIZE,
            );
            atomic_store_bytes(slot.add(PAYLOAD_OFFSET), data.as_ptr(), data.len());
            (*sh).seq.store(2 * idx, Ordering::Release);
        }
        self.write_idx().store(idx, Ordering::Release);
        self.notify();

        self.stats.record_published_stats(message.total_size());
        Ok(())
    }

    pub fn try_consume(&self, cursor: &mut BroadcastCursor) -> Result<Option<Message>> {
        let mut local = *cursor;
        let result = self.read_at(&mut local);
        let skipped = local.overruns - cursor.overruns;
        for _ in 0..skipped {
            self.stats.record_dropped();
        }
        *cursor = local;
        if let Ok(Some(ref message)) = result {
            let latency_ns = now_ns().saturating_sub(message.header.timestamp);
            self.stats.record_consumed(latency_ns);
        }
        result
    }

    pub fn try_peek(&self, cursor: &BroadcastCursor) -> Result<Option<Message>> {
        let mut local = *cursor;
        self.read_at(&mut local)
    }

    fn read_at(&self, cursor: &mut BroadcastCursor) -> Result<Option<Message>> {
        loop {
            let w = self.write_idx().load(Ordering::Acquire);
            if cursor.next > w {
                return Ok(None);
            }
            if w - cursor.next >= self.slot_count {
                self.skip_ahead(cursor, w);
                continue;
            }

            let slot = self.slot_ptr(cursor.next);
            let (header_bytes, payload) = unsafe {
                let sh = slot as *const SlotHeader;
                let s1 = (*sh).seq.load(Ordering::Acquire);
                let want = 2 * cursor.next;
                if s1 < want {
                    return Ok(None);
                }
                if s1 > want {
                    self.skip_ahead(cursor, self.write_idx().load(Ordering::Acquire));
                    continue;
                }

                let len = (*sh).len.load(Ordering::Relaxed) as usize;
                let mut header_bytes = [0u8; MessageHeader::SIZE];
                atomic_load_bytes(
                    slot.add(SLOT_HEADER_BYTES),
                    header_bytes.as_mut_ptr(),
                    MessageHeader::SIZE,
                );
                let payload = if len <= self.payload_cap {
                    let mut data = vec![0u8; len];
                    atomic_load_bytes(slot.add(PAYLOAD_OFFSET), data.as_mut_ptr(), len);
                    Some(data)
                } else {
                    None
                };

                fence(Ordering::Acquire);
                if (*sh).seq.load(Ordering::Relaxed) != s1 {
                    self.skip_ahead(cursor, self.write_idx().load(Ordering::Acquire));
                    continue;
                }
                match payload {
                    Some(data) => (header_bytes, data),
                    None => {
                        return Err(RenoirError::serialization(
                            "Broadcast slot length exceeds slot capacity",
                        ))
                    }
                }
            };

            let header: MessageHeader =
                unsafe { std::ptr::read_unaligned(header_bytes.as_ptr() as *const MessageHeader) };
            header.validate()?;
            if header.payload_length as usize != payload.len() {
                return Err(RenoirError::serialization(
                    "Broadcast message length mismatch",
                ));
            }
            cursor.next += 1;
            return Ok(Some(Message {
                header,
                payload: MessagePayload::Inline(payload),
            }));
        }
    }

    pub fn pending(&self, cursor: &BroadcastCursor) -> usize {
        let w = self.write_idx().load(Ordering::Acquire);
        if cursor.next > w {
            0
        } else {
            (w - cursor.next + 1).min(self.slot_count) as usize
        }
    }

    pub fn skip_to_latest(&self, cursor: &mut BroadcastCursor, keep: u64) -> u64 {
        let w = self.write_idx().load(Ordering::Acquire);
        let target = w.saturating_sub(keep.max(1) - 1).max(1);
        if target > cursor.next {
            let skipped = target - cursor.next;
            cursor.skipped += skipped;
            cursor.next = target;
            skipped
        } else {
            0
        }
    }

    fn skip_ahead(&self, cursor: &mut BroadcastCursor, write_idx: u64) {
        let keep = (self.slot_count * 3 / 4).max(1);
        let target = write_idx.saturating_sub(keep - 1).max(1);
        if target > cursor.next {
            cursor.overruns += target - cursor.next;
            cursor.next = target;
        }
    }

    fn slot_ptr(&self, idx: u64) -> *mut u8 {
        let i = ((idx - 1) % self.slot_count) as usize;
        unsafe { self.slots.as_ptr().add(i * self.stride) }
    }

    fn hdr(&self) -> &BroadcastHeader {
        unsafe { self.header.as_ref() }
    }

    fn write_idx(&self) -> &AtomicU64 {
        unsafe { &self.header.as_ref().write_idx }
    }
}

pub const BROADCAST_HEADER_PROBE_BYTES: usize = HEADER_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BroadcastControlInfo {
    pub slot_count: u64,
    pub payload_capacity: u64,
    pub published: u64,
    pub waiters: u32,
    pub session: u8,
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let raw: [u8; 8] = bytes.get(offset..offset + 8)?.try_into().ok()?;
    Some(u64::from_ne_bytes(raw))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_ne_bytes(raw))
}

impl BroadcastTopicRing {
    pub fn read_control_info(memory: &[u8]) -> Option<BroadcastControlInfo> {
        if read_u64(memory, std::mem::offset_of!(BroadcastHeader, magic))? != BROADCAST_RING_MAGIC {
            return None;
        }
        Some(BroadcastControlInfo {
            slot_count: read_u64(memory, std::mem::offset_of!(BroadcastHeader, slot_count))?,
            payload_capacity: read_u64(memory, std::mem::offset_of!(BroadcastHeader, payload_cap))?,
            published: read_u64(memory, std::mem::offset_of!(BroadcastHeader, write_idx))?,
            waiters: read_u32(memory, std::mem::offset_of!(BroadcastHeader, waiters))?,
            session: *memory.get(std::mem::offset_of!(BroadcastHeader, session))?,
        })
    }
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
