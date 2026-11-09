use std::sync::atomic::{fence, Ordering};

use crate::{
    error::{RenoirError, Result},
    topic::MessageHeader,
};

use super::{
    now_ns,
    types::{BroadcastCursor, BroadcastReservation, ReadInto},
    BroadcastTopicRing, SlotHeader, MAX_READ_ATTEMPTS, PAYLOAD_OFFSET, SLOT_HEADER_BYTES,
};
use crate::topic_rings::atomic_bytes::{atomic_load_bytes, atomic_store_bytes};

impl BroadcastTopicRing {
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
        self.notify();
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
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            if attempts > MAX_READ_ATTEMPTS {
                return Err(self.abandon_slot(cursor));
            }
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
}
