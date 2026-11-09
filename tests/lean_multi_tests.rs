use std::sync::Arc;
use std::thread;

use renoir::topic_rings::{lean_slot_bytes, LeanMode, LeanRead, LeanRing};

const MSG: usize = 16;
const PRODUCERS: u64 = 4;

fn make(slots: usize, mode: LeanMode) -> (Arc<LeanRing>, Vec<u64>) {
    let ring_bytes = lean_slot_bytes(MSG).unwrap() * slots;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring =
        unsafe { LeanRing::from_memory_mode(ptr, mem.len() * 8, ring_bytes, MSG, mode).unwrap() };
    (Arc::new(ring), mem)
}

fn encode(producer: u64, n: u64) -> [u8; MSG] {
    let v = (producer << 56) | n;
    let mut b = [0u8; MSG];
    b[..8].copy_from_slice(&v.to_le_bytes());
    b[8..].copy_from_slice(&v.to_le_bytes());
    b
}

fn decode(b: &[u8; MSG]) -> (u64, u64) {
    let lo = u64::from_le_bytes(b[..8].try_into().unwrap());
    let hi = u64::from_le_bytes(b[8..].try_into().unwrap());
    assert_eq!(lo, hi, "torn read");
    (lo >> 56, lo & ((1 << 56) - 1))
}

#[test]
fn every_message_from_every_producer_arrives_in_per_producer_order() {
    let per = if cfg!(miri) { 20 } else { 20_000 };
    let slots = (PRODUCERS * per).next_power_of_two() as usize;
    let (ring, _mem) = make(slots, LeanMode::MultiProducer);
    let mut cursor = ring.cursor_oldest();
    let handles: Vec<_> = (0..PRODUCERS)
        .map(|p| {
            let ring = ring.clone();
            thread::spawn(move || {
                let mut publisher = ring.publisher();
                for n in 1..=per {
                    publisher.publish(&encode(p, n)).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let mut last = [0u64; PRODUCERS as usize];
    let mut out = [0u8; MSG];
    let mut total = 0;
    while ring.try_read(&mut cursor, &mut out).unwrap() == LeanRead::Message {
        let (p, n) = decode(&out);
        assert_eq!(n, last[p as usize] + 1, "producer {p} out of order");
        last[p as usize] = n;
        total += 1;
    }
    assert_eq!(total, PRODUCERS * per);
    assert_eq!(cursor.lost(), 0);
}

#[test]
fn concurrent_reader_sees_intact_ordered_messages_while_ring_laps() {
    let per = if cfg!(miri) { 40 } else { 50_000 };
    let (ring, _mem) = make(64, LeanMode::MultiProducer);
    let mut cursor = ring.cursor_latest();
    let handles: Vec<_> = (0..PRODUCERS)
        .map(|p| {
            let ring = ring.clone();
            thread::spawn(move || {
                let mut publisher = ring.publisher();
                for n in 1..=per {
                    publisher.publish(&encode(p, n)).unwrap();
                }
            })
        })
        .collect();
    let mut last = [0u64; PRODUCERS as usize];
    let mut out = [0u8; MSG];
    let mut seen = 0u64;
    while !handles.iter().all(|h| h.is_finished()) || ring.pending(&cursor) > 0 {
        if let Ok(LeanRead::Message) = ring.try_read(&mut cursor, &mut out) {
            let (p, n) = decode(&out);
            assert!(n > last[p as usize], "producer {p} went backwards");
            last[p as usize] = n;
            seen += 1;
        }
    }
    for h in handles {
        h.join().unwrap();
    }
    assert!(seen > 0);
    assert_eq!(seen + cursor.lost(), PRODUCERS * per);
}

#[test]
fn producer_mode_mismatch_is_rejected() {
    let ring_bytes = lean_slot_bytes(MSG).unwrap() * 16;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let size = mem.len() * 8;
    unsafe {
        LeanRing::from_memory_mode(ptr, size, ring_bytes, MSG, LeanMode::MultiProducer).unwrap();
        assert!(
            LeanRing::from_memory_mode(ptr, size, ring_bytes, MSG, LeanMode::SingleProducer)
                .is_err()
        );
        LeanRing::from_memory_mode(ptr, size, ring_bytes, MSG, LeanMode::MultiProducer).unwrap();
    }
}
