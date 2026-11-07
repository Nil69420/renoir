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

use super::spsc::{SESSION_DIRTY, SESSION_MARKER_OFFSET};

pub const BROADCAST_RING_MAGIC: u64 = 0x4243_4153_5452_4E47;

const HEADER_BYTES: usize = 64;
const SLOT_HEADER_BYTES: usize = 16;
const SLOT_ALIGN: usize = 64;
const MESSAGE_AREA_BYTES: usize = 64;
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

const _: () = assert!(std::mem::size_of::<SlotHeader>() == SLOT_HEADER_BYTES);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadInto {
    Empty,
    TooSmall(usize),
    Message {
        len: usize,
        sequence: u64,
        timestamp_ns: u64,
    },
}

#[derive(Debug)]
pub struct BroadcastReservation {
    idx: u64,
    payload: *mut u8,
    capacity: usize,
}

unsafe impl Send for BroadcastReservation {}

impl BroadcastReservation {
    pub fn payload_ptr(&self) -> *mut u8 {
        self.payload
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BroadcastCursor {
    next: u64,
    overruns: u64,
}

impl BroadcastCursor {
    pub fn overruns(&self) -> u64 {
        self.overruns
    }

    pub fn next_index(&self) -> u64 {
        self.next
    }
}

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
        let existing_magic = unsafe { (*header_ptr.as_ptr()).magic };
        if existing_magic == BROADCAST_RING_MAGIC {
            let (count, str_, cap) = unsafe {
                let h = &*header_ptr.as_ptr();
                (h.slot_count, h.stride, h.payload_cap)
            };
            if count != slot_count || str_ != stride || cap != payload_cap {
                return Err(RenoirError::invalid_parameter(
                    "ring",
                    "Existing broadcast ring has a different geometry",
                ));
            }
        } else {
            unsafe {
                header_ptr.as_ptr().write(BroadcastHeader {
                    magic: BROADCAST_RING_MAGIC,
                    slot_count,
                    session: SESSION_DIRTY,
                    stride,
                    payload_cap,
                    write_idx: AtomicU64::new(0),
                });
            }
        }

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
        }
    }

    pub fn cursor_oldest(&self) -> BroadcastCursor {
        let w = self.published_count();
        BroadcastCursor {
            next: w.saturating_sub(self.slot_count - 1).max(1),
            overruns: 0,
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

        self.stats.record_published_stats(message.total_size());
        Ok(())
    }

    pub fn reserve(&self, len: usize) -> Result<BroadcastReservation> {
        if len > self.payload_cap {
            return Err(RenoirError::invalid_parameter(
                "size",
                "Reservation exceeds broadcast slot capacity",
            ));
        }
        if self
            .reserved
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(RenoirError::invalid_parameter(
                "reservation",
                "A reservation is already outstanding on this publisher",
            ));
        }
        let idx = self.write_idx().load(Ordering::Relaxed) + 1;
        let slot = self.slot_ptr(idx);
        unsafe {
            let sh = slot as *mut SlotHeader;
            (*sh).seq.store(2 * idx - 1, Ordering::Relaxed);
            fence(Ordering::Release);
            (*sh).len.store(0, Ordering::Relaxed);
        }
        Ok(BroadcastReservation {
            idx,
            payload: unsafe { slot.add(PAYLOAD_OFFSET) },
            capacity: self.payload_cap,
        })
    }

    pub fn commit(
        &self,
        reservation: BroadcastReservation,
        header: &MessageHeader,
        len: usize,
    ) -> Result<()> {
        if len > reservation.capacity {
            self.reserved.store(false, Ordering::Release);
            return Err(RenoirError::invalid_parameter(
                "len",
                "Committed length exceeds reservation",
            ));
        }
        let idx = reservation.idx;
        let slot = self.slot_ptr(idx);
        unsafe {
            let sh = slot as *mut SlotHeader;
            (*sh).len.store(len as u32, Ordering::Relaxed);
            atomic_store_bytes(
                slot.add(SLOT_HEADER_BYTES),
                header as *const MessageHeader as *const u8,
                MessageHeader::SIZE,
            );
            (*sh).seq.store(2 * idx, Ordering::Release);
        }
        self.write_idx().store(idx, Ordering::Release);
        self.reserved.store(false, Ordering::Release);
        self.stats.record_published_stats(MessageHeader::SIZE + len);
        Ok(())
    }

    pub fn discard(&self, _reservation: BroadcastReservation) {
        self.reserved.store(false, Ordering::Release);
    }

    pub fn read_into(&self, cursor: &mut BroadcastCursor, dst: &mut [u8]) -> Result<ReadInto> {
        let mut local = *cursor;
        let result = self.read_into_at(&mut local, dst);
        let skipped = local.overruns - cursor.overruns;
        for _ in 0..skipped {
            self.stats.record_dropped();
        }
        *cursor = local;
        if let Ok(ReadInto::Message { timestamp_ns, .. }) = result {
            self.stats
                .record_consumed(now_ns().saturating_sub(timestamp_ns));
        }
        result
    }

    fn read_into_at(&self, cursor: &mut BroadcastCursor, dst: &mut [u8]) -> Result<ReadInto> {
        loop {
            let w = self.write_idx().load(Ordering::Acquire);
            if cursor.next > w {
                return Ok(ReadInto::Empty);
            }
            if w - cursor.next >= self.slot_count {
                self.skip_ahead(cursor, w);
                continue;
            }
            let slot = self.slot_ptr(cursor.next);
            let (header_bytes, len) = unsafe {
                let sh = slot as *const SlotHeader;
                let s1 = (*sh).seq.load(Ordering::Acquire);
                let want = 2 * cursor.next;
                if s1 < want {
                    return Ok(ReadInto::Empty);
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
                let fits = len <= self.payload_cap && len <= dst.len();
                if fits {
                    atomic_load_bytes(slot.add(PAYLOAD_OFFSET), dst.as_mut_ptr(), len);
                }
                fence(Ordering::Acquire);
                if (*sh).seq.load(Ordering::Relaxed) != s1 {
                    self.skip_ahead(cursor, self.write_idx().load(Ordering::Acquire));
                    continue;
                }
                if len > self.payload_cap {
                    return Err(RenoirError::serialization(
                        "Broadcast slot length exceeds slot capacity",
                    ));
                }
                if !fits {
                    return Ok(ReadInto::TooSmall(len));
                }
                (header_bytes, len)
            };
            let header: MessageHeader =
                unsafe { std::ptr::read_unaligned(header_bytes.as_ptr() as *const MessageHeader) };
            header.validate()?;
            if header.payload_length as usize != len {
                return Err(RenoirError::serialization(
                    "Broadcast message length mismatch",
                ));
            }
            cursor.next += 1;
            return Ok(ReadInto::Message {
                len,
                sequence: header.sequence,
                timestamp_ns: header.timestamp,
            });
        }
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

    fn write_idx(&self) -> &AtomicU64 {
        unsafe { &self.header.as_ref().write_idx }
    }
}

fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

unsafe fn atomic_store_bytes(dst: *mut u8, src: *const u8, len: usize) {
    let words = len / 8;
    for i in 0..words {
        let w = std::ptr::read_unaligned(src.add(i * 8) as *const u64);
        (*(dst.add(i * 8) as *const AtomicU64)).store(w, Ordering::Relaxed);
    }
    let rem = len % 8;
    if rem != 0 {
        let mut tail = [0u8; 8];
        std::ptr::copy_nonoverlapping(src.add(words * 8), tail.as_mut_ptr(), rem);
        (*(dst.add(words * 8) as *const AtomicU64))
            .store(u64::from_ne_bytes(tail), Ordering::Relaxed);
    }
}

unsafe fn atomic_load_bytes(src: *const u8, dst: *mut u8, len: usize) {
    let words = len / 8;
    for i in 0..words {
        let w = (*(src.add(i * 8) as *const AtomicU64)).load(Ordering::Relaxed);
        std::ptr::write_unaligned(dst.add(i * 8) as *mut u64, w);
    }
    let rem = len % 8;
    if rem != 0 {
        let tail = (*(src.add(words * 8) as *const AtomicU64))
            .load(Ordering::Relaxed)
            .to_ne_bytes();
        std::ptr::copy_nonoverlapping(tail.as_ptr(), dst.add(words * 8), rem);
    }
}
