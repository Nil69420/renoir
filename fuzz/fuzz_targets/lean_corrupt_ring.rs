#![no_main]
use libfuzzer_sys::fuzz_target;
use renoir::topic_rings::{lean_slot_bytes, LeanRing, LEAN_HEADER_BYTES};

const SLOTS: usize = 8;
const MSG: usize = 100;

fuzz_target!(|data: &[u8]| {
    let slot = lean_slot_bytes(MSG).unwrap();
    let ring_bytes = slot * SLOTS;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut memory = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(memory.as_mut_ptr() as *mut u8).unwrap();
    let ring = match unsafe { LeanRing::from_memory(ptr, memory.len() * 8, ring_bytes, MSG) } {
        Ok(r) => std::sync::Arc::new(r),
        Err(_) => return,
    };
    let bytes: &mut [u8] =
        unsafe { std::slice::from_raw_parts_mut(memory.as_mut_ptr() as *mut u8, memory.len() * 8) };
    let n = data.len().min(bytes.len() - LEAN_HEADER_BYTES);
    bytes[LEAN_HEADER_BYTES..LEAN_HEADER_BYTES + n].copy_from_slice(&data[..n]);
    if let Some(&mode) = data.first() {
        let published = u64::from(mode % 40);
        bytes[64..72].copy_from_slice(&published.to_ne_bytes());
    }
    let mut cursor = ring.cursor_oldest();
    let mut dst = [0u8; MSG];
    for _ in 0..16 {
        let _ = ring.pending(&cursor);
        match ring.try_read(&mut cursor, &mut dst) {
            Ok(renoir::topic_rings::LeanRead::Empty) | Err(_) => break,
            Ok(_) => {}
        }
    }
    let _ = ring.skip_to_latest(&mut cursor, 3);
    let mut publisher = ring.publisher();
    let _ = publisher.publish(&dst);
    let _ = ring.try_read(&mut cursor, &mut dst);
});
