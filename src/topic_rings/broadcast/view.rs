use std::sync::atomic::{fence, Ordering};

use crate::{
    error::{RenoirError, Result},
    topic::MessageHeader,
};

use super::{
    types::{BroadcastCursor, BroadcastView, ViewRead},
    BroadcastTopicRing, SlotHeader, MAX_READ_ATTEMPTS, PAYLOAD_OFFSET, SLOT_HEADER_BYTES,
};
use crate::topic_rings::atomic_bytes::atomic_load_bytes;

impl BroadcastTopicRing {
    pub fn view(&self, cursor: &mut BroadcastCursor) -> Result<ViewRead> {
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            if attempts > MAX_READ_ATTEMPTS {
                return Err(self.abandon_slot(cursor));
            }
            let w = self.write_idx().load(Ordering::Acquire);
            if cursor.next > w {
                return Ok(ViewRead::Empty);
            }
            if w - cursor.next >= self.slot_count {
                self.skip_ahead(cursor, w);
                continue;
            }
            let idx = cursor.next;
            let slot = self.slot_ptr(idx);
            let sh = slot as *const SlotHeader;
            let want = 2 * idx;
            let s1 = unsafe { (*sh).seq.load(Ordering::Acquire) };
            if s1 < want {
                return Ok(ViewRead::Empty);
            }
            if s1 > want {
                self.skip_ahead(cursor, self.write_idx().load(Ordering::Acquire));
                continue;
            }
            let len = unsafe { (*sh).len.load(Ordering::Relaxed) } as usize;
            let mut header_bytes = [0u8; MessageHeader::SIZE];
            unsafe {
                atomic_load_bytes(
                    slot.add(SLOT_HEADER_BYTES),
                    header_bytes.as_mut_ptr(),
                    MessageHeader::SIZE,
                );
            }
            fence(Ordering::Acquire);
            if unsafe { (*sh).seq.load(Ordering::Relaxed) } != s1 {
                self.skip_ahead(cursor, self.write_idx().load(Ordering::Acquire));
                continue;
            }
            if len > self.payload_cap {
                return Err(RenoirError::serialization(
                    "Broadcast slot length exceeds slot capacity",
                ));
            }
            let header: MessageHeader =
                unsafe { std::ptr::read_unaligned(header_bytes.as_ptr().cast::<MessageHeader>()) };
            header.validate()?;
            if header.payload_length as usize != len {
                return Err(RenoirError::serialization(
                    "Broadcast message length mismatch",
                ));
            }
            return Ok(ViewRead::View(BroadcastView {
                ptr: unsafe { slot.add(PAYLOAD_OFFSET) },
                len,
                sequence: header.sequence,
                timestamp_ns: header.timestamp,
                index: idx,
            }));
        }
    }

    pub fn view_is_intact(&self, view: &BroadcastView) -> bool {
        fence(Ordering::Acquire);
        let sh = self.slot_ptr(view.index) as *const SlotHeader;
        unsafe { (*sh).seq.load(Ordering::Relaxed) == 2 * view.index }
    }

    pub fn release_view(&self, cursor: &mut BroadcastCursor, view: &BroadcastView) -> Result<bool> {
        if cursor.next != view.index {
            return Err(RenoirError::invalid_parameter(
                "view",
                "View does not belong to the next unread message",
            ));
        }
        let intact = self.view_is_intact(view);
        cursor.next = view.index + 1;
        if !intact {
            cursor.overruns += 1;
            self.stats.record_dropped();
        }
        Ok(intact)
    }
}
