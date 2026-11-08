use std::sync::atomic::{fence, AtomicU64, Ordering};

use super::{
    atomic_store_bytes, multi::fill_slot_multi, LeanCursor, LeanGatedCursor, LeanPublisher,
    LeanRead, LeanRing, SEQ_BYTES,
};
use crate::error::{RenoirError, Result};

impl LeanPublisher {
    pub fn publish_batch(&mut self, data: &[u8]) -> Result<usize> {
        let msg = self.ring.msg_size;
        if data.is_empty() || !data.len().is_multiple_of(msg) {
            return Err(RenoirError::invalid_parameter(
                "payload",
                "Batch must be a non-empty multiple of the topic message size",
            ));
        }
        let count = data.len() / msg;
        if count > self.ring.slot_count() {
            return Err(RenoirError::invalid_parameter(
                "payload",
                "Batch is larger than the ring",
            ));
        }
        match (self.ring.multi, self.ring.gated) {
            (false, false) => Ok(self.batch_single(data, count)),
            (false, true) => self.batch_single_gated(data, count),
            (true, false) => self.batch_multi(data, count),
            (true, true) => self.batch_multi_gated(data, count),
        }
    }

    fn write_single(&self, idx: u64, data: &[u8]) {
        let slot = self.ring.slot_ptr(idx);
        unsafe {
            let seq = &*(slot as *const AtomicU64);
            seq.store(2 * idx - 1, Ordering::Relaxed);
            fence(Ordering::Release);
            atomic_store_bytes(slot.add(SEQ_BYTES), data.as_ptr(), data.len());
            seq.store(2 * idx, Ordering::Release);
        }
    }

    fn batch_single(&mut self, data: &[u8], count: usize) -> usize {
        let msg = self.ring.msg_size;
        for (i, chunk) in data.chunks_exact(msg).take(count).enumerate() {
            self.write_single(self.next + i as u64, chunk);
        }
        let last = self.next + count as u64 - 1;
        self.ring.write_idx().store(last, Ordering::Release);
        self.next = last + 1;
        count
    }

    fn batch_single_gated(&mut self, data: &[u8], count: usize) -> Result<usize> {
        let msg = self.ring.msg_size;
        let mut written = 0usize;
        for chunk in data.chunks_exact(msg).take(count) {
            let idx = self.next + written as u64;
            if self.gate_admits(idx).is_err() {
                break;
            }
            self.write_single(idx, chunk);
            written += 1;
        }
        if written == 0 {
            return Err(RenoirError::buffer_full("lean ring"));
        }
        let last = self.next + written as u64 - 1;
        self.ring.write_idx().store(last, Ordering::Release);
        self.next = last + 1;
        Ok(written)
    }

    fn batch_multi(&mut self, data: &[u8], count: usize) -> Result<usize> {
        let msg = self.ring.msg_size;
        let first = self.claim_range(count as u64);
        for (i, chunk) in data.chunks_exact(msg).take(count).enumerate() {
            if let Err(e) = fill_slot_multi(&self.ring, first + i as u64, chunk) {
                self.end_claim();
                return Err(e);
            }
        }
        self.end_claim();
        self.ring
            .write_idx()
            .fetch_max(first + count as u64 - 1, Ordering::Release);
        Ok(count)
    }

    fn batch_multi_gated(&mut self, data: &[u8], count: usize) -> Result<usize> {
        let msg = self.ring.msg_size;
        let mut written = 0usize;
        for chunk in data.chunks_exact(msg).take(count) {
            if self.publish_multi(chunk).is_err() {
                break;
            }
            written += 1;
        }
        if written == 0 {
            return Err(RenoirError::buffer_full("lean ring"));
        }
        Ok(written)
    }
}

impl LeanRing {
    pub fn read_batch(&self, cursor: &mut LeanCursor, out: &mut [u8]) -> Result<usize> {
        let msg = self.msg_size;
        if out.len() < msg {
            return Err(RenoirError::invalid_parameter(
                "buffer",
                "Output buffer smaller than the lean message size",
            ));
        }
        let mut read = 0usize;
        for chunk in out.chunks_exact_mut(msg) {
            match self.try_read(cursor, chunk) {
                Ok(LeanRead::Message) => read += 1,
                Ok(LeanRead::Empty) => break,
                Err(e) if read == 0 => return Err(e),
                Err(_) => break,
            }
        }
        Ok(read)
    }

    pub fn read_batch_gated(&self, cursor: &mut LeanGatedCursor, out: &mut [u8]) -> Result<usize> {
        let read = self.read_batch(&mut cursor.cursor, out);
        if matches!(read, Ok(n) if n > 0) {
            self.sync_gate(cursor);
        }
        read
    }
}
