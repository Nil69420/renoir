use std::ptr::NonNull;
use std::sync::Arc;

use renoir::topic_rings::{lean_slot_bytes, LeanClock, LeanMode, LeanRing, STAMP_BYTES};

const DATA: usize = 8;
const PLAIN: usize = 16;

fn arg(position: usize, default: u64) -> u64 {
    std::env::args()
        .nth(position)
        .and_then(|a| a.parse().ok())
        .unwrap_or(default)
}

fn make(mem: &mut [u64], ring_bytes: usize, stamped: bool) -> Arc<LeanRing> {
    let start = mem.as_mut_ptr().cast::<u8>();
    let pad = start.align_offset(64);
    let ptr = NonNull::new(unsafe { start.add(pad) }).unwrap();
    let total = mem.len() * 8 - pad;
    Arc::new(unsafe {
        if stamped {
            LeanRing::from_memory_stamped(
                ptr,
                total,
                ring_bytes,
                DATA,
                LeanMode::SingleProducer,
                LeanClock::Nanoseconds,
            )
        } else {
            LeanRing::from_memory_mode(ptr, total, ring_bytes, PLAIN, LeanMode::SingleProducer)
        }
        .unwrap()
    })
}

fn main() {
    let messages = arg(1, 12);
    let polls = arg(2, 96);
    let stamped = arg(3, 0) == 0;
    let msg = if stamped { DATA + STAMP_BYTES } else { PLAIN };
    let ring_bytes = lean_slot_bytes(msg).unwrap() * 2;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let mut mem = vec![0u64; total / 8 + 16];
    let ring = make(&mut mem, ring_bytes, stamped);
    let writer_ring = ring.clone();
    let writer = std::thread::spawn(move || {
        let mut p = writer_ring.publisher();
        for i in 1..=messages {
            if stamped {
                p.publish_at(i, &i.to_ne_bytes()).unwrap();
            } else {
                let mut payload = [0u8; PLAIN];
                payload[..8].copy_from_slice(&i.to_ne_bytes());
                payload[8..].copy_from_slice(&i.to_ne_bytes());
                p.publish(&payload).unwrap();
            }
        }
    });
    let mut cursor = ring.cursor_oldest();
    let mut out = [0u8; PLAIN];
    for _ in 0..polls {
        if stamped {
            if let Ok(Some(stamp)) = ring.try_read_stamped(&mut cursor, &mut out[..DATA]) {
                assert_eq!(
                    stamp,
                    u64::from_ne_bytes(out[..8].try_into().unwrap()),
                    "stamp and payload from different messages"
                );
            }
        } else if let Ok(renoir::topic_rings::LeanRead::Message) =
            ring.try_read(&mut cursor, &mut out)
        {
            assert_eq!(out[..8], out[8..], "torn payload");
        }
    }
    writer.join().unwrap();
    drop(mem);
}
