#![no_main]
use libfuzzer_sys::fuzz_target;
use renoir::topic::TopicStats;
use renoir::topic_rings::{BroadcastTopicRing, ReadInto};
use std::sync::Arc;

const SLOTS: usize = 8;
const CAP: usize = 200;

fuzz_target!(|data: &[u8]| {
    let ring_bytes = BroadcastTopicRing::slot_stride(CAP) * SLOTS;
    let total = BroadcastTopicRing::required_memory_size(ring_bytes, CAP);
    let mut memory = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(memory.as_mut_ptr() as *mut u8).unwrap();
    let ring = match unsafe {
        BroadcastTopicRing::from_memory(ptr, memory.len() * 8, ring_bytes, CAP, Arc::new(TopicStats::default()))
    } {
        Ok(r) => r,
        Err(_) => return,
    };
    let bytes: &mut [u8] =
        unsafe { std::slice::from_raw_parts_mut(memory.as_mut_ptr() as *mut u8, memory.len() * 8) };
    let skip = 64usize;
    let n = data.len().min(bytes.len() - skip);
    bytes[skip..skip + n].copy_from_slice(&data[..n]);
    let stride = BroadcastTopicRing::slot_stride(CAP);
    if let Some(&mode) = data.first() {
        let published = u64::from(mode % 24);
        bytes[40..48].copy_from_slice(&published.to_ne_bytes());
        for idx in 1..=published.min(SLOTS as u64) {
            let slot = skip + ((idx - 1) as usize % SLOTS) * stride;
            if mode & 0x40 == 0 {
                bytes[slot..slot + 8].copy_from_slice(&(2 * idx).to_ne_bytes());
            }
            if mode & 0x20 == 0 {
                let magic = 0x5245_4E4F_4952_4D53u64.to_ne_bytes();
                bytes[slot + 16..slot + 24].copy_from_slice(&magic);
            }
        }
    }
    let mut cursor = ring.cursor_oldest();
    let mut dst = [0u8; CAP];
    for _ in 0..4 {
        let _ = ring.try_peek(&cursor);
        let _ = ring.pending(&cursor);
        match ring.read_into(&mut cursor, &mut dst) {
            Ok(ReadInto::Empty) | Err(_) => break,
            Ok(_) => {}
        }
        let _ = ring.try_consume(&mut cursor);
    }
    let _ = ring.skip_to_latest(&mut cursor, 3);
});
