use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use renoir::topic_rings::{lean_slot_bytes, LeanMode, LeanRead, LeanRing, LEAN_MAX_CONSUMERS};

const MSG: usize = 16;

fn make(slots: usize, mode: LeanMode) -> (Arc<LeanRing>, Vec<u64>) {
    let ring_bytes = lean_slot_bytes(MSG).unwrap() * slots;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring =
        unsafe { LeanRing::from_memory_mode(ptr, mem.len() * 8, ring_bytes, MSG, mode).unwrap() };
    (Arc::new(ring), mem)
}

fn payload(producer: u64, n: u64) -> [u8; MSG] {
    let v = (producer << 56) | n;
    let mut b = [0u8; MSG];
    b[..8].copy_from_slice(&v.to_le_bytes());
    b[8..].copy_from_slice(&v.to_le_bytes());
    b
}

fn decode(b: &[u8; MSG]) -> (u64, u64) {
    let lo = u64::from_le_bytes(b[..8].try_into().unwrap());
    assert_eq!(lo, u64::from_le_bytes(b[8..].try_into().unwrap()), "torn");
    (lo >> 56, lo & ((1 << 56) - 1))
}

#[test]
fn full_ring_refuses_to_overwrite_unread_messages() {
    let (ring, _mem) = make(8, LeanMode::SingleProducerGated);
    let mut cursor = ring.register_consumer(false).unwrap();
    let mut publisher = ring.publisher();
    let slots = ring.slot_count() as u64;
    for n in 1..=slots {
        publisher.publish(&payload(0, n)).unwrap();
    }
    assert!(publisher.publish(&payload(0, slots + 1)).is_err());
    let mut out = [0u8; MSG];
    assert_eq!(
        ring.try_read_gated(&mut cursor, &mut out).unwrap(),
        LeanRead::Message
    );
    assert_eq!(decode(&out), (0, 1));
    publisher.publish(&payload(0, slots + 1)).unwrap();
    assert!(publisher.publish(&payload(0, slots + 2)).is_err());
    for n in 2..=slots + 1 {
        assert_eq!(
            ring.try_read_gated(&mut cursor, &mut out).unwrap(),
            LeanRead::Message
        );
        assert_eq!(decode(&out), (0, n));
    }
    assert_eq!(cursor.lost(), 0);
}

#[test]
fn ring_without_registered_consumers_never_blocks() {
    let (ring, _mem) = make(8, LeanMode::SingleProducerGated);
    let mut publisher = ring.publisher();
    for n in 1..=100 {
        publisher.publish(&payload(0, n)).unwrap();
    }
}

#[test]
fn slowest_consumer_gates_the_producer() {
    let (ring, _mem) = make(8, LeanMode::SingleProducerGated);
    let mut fast = ring.register_consumer(false).unwrap();
    let mut slow = ring.register_consumer(false).unwrap();
    let mut publisher = ring.publisher();
    let mut out = [0u8; MSG];
    for n in 1..=8 {
        publisher.publish(&payload(0, n)).unwrap();
        ring.try_read_gated(&mut fast, &mut out).unwrap();
    }
    assert!(publisher.publish(&payload(0, 9)).is_err());
    ring.try_read_gated(&mut slow, &mut out).unwrap();
    publisher.publish(&payload(0, 9)).unwrap();
}

#[test]
fn consumer_that_stops_reading_is_evicted_and_resyncs() {
    let (ring, _mem) = make(8, LeanMode::SingleProducerGated);
    ring.set_gate_timeout(Duration::from_millis(20));
    let mut stuck = ring.register_consumer(false).unwrap();
    let mut publisher = ring.publisher();
    for n in 1..=8 {
        publisher.publish(&payload(0, n)).unwrap();
    }
    assert!(publisher.publish(&payload(0, 9)).is_err());
    let start = Instant::now();
    let mut next = 9;
    while publisher.publish(&payload(0, next)).is_err() {
        assert!(start.elapsed() < Duration::from_secs(5), "never evicted");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(start.elapsed() >= Duration::from_millis(15));
    next += 1;
    for n in next..next + 20 {
        publisher.publish(&payload(0, n)).unwrap();
    }
    let mut out = [0u8; MSG];
    let mut got = Vec::new();
    while ring.try_read_gated(&mut stuck, &mut out).unwrap() == LeanRead::Message {
        got.push(decode(&out).1);
    }
    assert!(stuck.lost() > 0);
    assert!(stuck.is_registered());
    assert!(got.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(*got.last().unwrap(), next + 19);
}

#[test]
fn consumer_table_is_bounded_and_slots_are_reusable() {
    let (ring, _mem) = make(8, LeanMode::SingleProducerGated);
    let mut cursors: Vec<_> = (0..LEAN_MAX_CONSUMERS)
        .map(|_| ring.register_consumer(false).unwrap())
        .collect();
    assert!(ring.register_consumer(false).is_err());
    let mut first = cursors.remove(0);
    ring.unregister_consumer(&mut first);
    assert!(!first.is_registered());
    assert!(ring.register_consumer(false).is_ok());
}

#[test]
fn registration_requires_a_gated_ring() {
    let (ring, _mem) = make(8, LeanMode::SingleProducer);
    assert!(ring.register_consumer(false).is_err());
}

fn lossless_run(mode: LeanMode, producers: u64, per: u64, slots: usize) {
    let (ring, _mem) = make(slots, mode);
    let mut cursor = ring.register_consumer(true).unwrap();
    let handles: Vec<_> = (0..producers)
        .map(|p| {
            let ring = ring.clone();
            thread::spawn(move || {
                let mut publisher = ring.publisher();
                for n in 1..=per {
                    let data = payload(p, n);
                    while publisher.publish(&data).is_err() {
                        std::hint::spin_loop();
                    }
                }
            })
        })
        .collect();
    let mut last = vec![0u64; producers as usize];
    let mut out = [0u8; MSG];
    let mut total = 0u64;
    let deadline = Instant::now() + Duration::from_secs(60);
    while total < producers * per {
        assert!(Instant::now() < deadline, "consumer starved at {total}");
        if ring.try_read_gated(&mut cursor, &mut out).unwrap() == LeanRead::Message {
            let (p, n) = decode(&out);
            assert_eq!(n, last[p as usize] + 1, "producer {p} lost or reordered");
            last[p as usize] = n;
            total += 1;
        }
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(cursor.lost(), 0);
}

#[test]
fn single_producer_gated_is_lossless_under_load() {
    let per = if cfg!(miri) { 100 } else { 300_000 };
    lossless_run(LeanMode::SingleProducerGated, 1, per, 64);
}

#[test]
fn multi_producer_gated_is_lossless_under_load() {
    let per = if cfg!(miri) { 40 } else { 100_000 };
    lossless_run(LeanMode::MultiProducerGated, 3, per, 64);
}
