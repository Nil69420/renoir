use std::ptr::NonNull;
use std::sync::Arc;

use renoir::topic_rings::{lean_slot_bytes, LeanClock, LeanMode, LeanRing, STAMP_BYTES};

const DATA: usize = 8;
const MESSAGES: u64 = 12;

fn main() {
    let msg = DATA + STAMP_BYTES;
    let slot = lean_slot_bytes(msg).unwrap();
    let ring_bytes = slot * 2;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let mut mem = vec![0u64; total / 8 + 8];
    let base = (mem.as_mut_ptr() as usize).div_ceil(64) * 64;
    let ptr = NonNull::new(base as *mut u8).unwrap();
    let ring = Arc::new(unsafe {
        LeanRing::from_memory_stamped(
            ptr,
            total,
            ring_bytes,
            DATA,
            LeanMode::SingleProducer,
            LeanClock::Nanoseconds,
        )
        .unwrap()
    });
    let writer_ring = ring.clone();
    let writer = std::thread::spawn(move || {
        let mut p = writer_ring.publisher();
        for i in 1..=MESSAGES {
            p.publish_at(i, &i.to_ne_bytes()).unwrap();
        }
    });
    let mut cursor = ring.cursor_oldest();
    let mut out = [0u8; DATA];
    for _ in 0..(MESSAGES * 8) {
        if let Ok(Some(stamp)) = ring.try_read_stamped(&mut cursor, &mut out) {
            assert_eq!(
                stamp,
                u64::from_ne_bytes(out),
                "stamp and payload from different messages"
            );
        }
    }
    writer.join().unwrap();
    drop(mem);
}
