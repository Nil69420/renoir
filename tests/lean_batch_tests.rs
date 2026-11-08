use std::sync::Arc;
use std::thread;

use renoir::topic_rings::{lean_slot_bytes, LeanMode, LeanRing};

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

fn batch(producer: u64, first: u64, count: u64) -> Vec<u8> {
    let mut v = Vec::new();
    for n in first..first + count {
        let value = (producer << 56) | n;
        v.extend_from_slice(&value.to_le_bytes());
        v.extend_from_slice(&value.to_le_bytes());
    }
    v
}

fn decode(chunk: &[u8]) -> (u64, u64) {
    let lo = u64::from_le_bytes(chunk[..8].try_into().unwrap());
    assert_eq!(
        lo,
        u64::from_le_bytes(chunk[8..].try_into().unwrap()),
        "torn"
    );
    (lo >> 56, lo & ((1 << 56) - 1))
}

#[test]
fn single_producer_batch_round_trips_in_order() {
    let (ring, _mem) = make(64, LeanMode::SingleProducer);
    let mut cursor = ring.cursor_oldest();
    let mut publisher = ring.publisher();
    assert_eq!(publisher.publish_batch(&batch(0, 1, 10)).unwrap(), 10);
    assert_eq!(publisher.publish_batch(&batch(0, 11, 5)).unwrap(), 5);
    let mut out = vec![0u8; MSG * 8];
    let mut seen = Vec::new();
    loop {
        let n = ring.read_batch(&mut cursor, &mut out).unwrap();
        if n == 0 {
            break;
        }
        assert!(n <= 8);
        for chunk in out.chunks_exact(MSG).take(n) {
            seen.push(decode(chunk).1);
        }
    }
    assert_eq!(seen, (1..=15).collect::<Vec<_>>());
    assert_eq!(cursor.lost(), 0);
}

#[test]
fn malformed_batches_are_rejected() {
    let (ring, _mem) = make(8, LeanMode::SingleProducer);
    let mut publisher = ring.publisher();
    assert!(publisher.publish_batch(&[]).is_err());
    assert!(publisher.publish_batch(&[0u8; MSG + 1]).is_err());
    assert!(publisher.publish_batch(&batch(0, 1, 9)).is_err());
    let mut cursor = ring.cursor_latest();
    assert!(ring.read_batch(&mut cursor, &mut [0u8; MSG - 1]).is_err());
}

#[test]
fn gated_batch_publishes_only_what_fits() {
    let (ring, _mem) = make(8, LeanMode::SingleProducerGated);
    let mut consumer = ring.register_consumer(false).unwrap();
    let mut publisher = ring.publisher();
    assert_eq!(publisher.publish_batch(&batch(0, 1, 6)).unwrap(), 6);
    assert_eq!(publisher.publish_batch(&batch(0, 7, 6)).unwrap(), 2);
    assert!(publisher.publish_batch(&batch(0, 9, 4)).is_err());
    let mut out = vec![0u8; MSG * 4];
    assert_eq!(ring.read_batch_gated(&mut consumer, &mut out).unwrap(), 4);
    assert_eq!(publisher.publish_batch(&batch(0, 9, 4)).unwrap(), 4);
    let mut seen = Vec::new();
    loop {
        let n = ring.read_batch_gated(&mut consumer, &mut out).unwrap();
        if n == 0 {
            break;
        }
        for chunk in out.chunks_exact(MSG).take(n) {
            seen.push(decode(chunk).1);
        }
    }
    assert_eq!(seen, (5..=12).collect::<Vec<_>>());
    assert_eq!(consumer.lost(), 0);
}

fn multi_run(mode: LeanMode) {
    let producers = 3u64;
    let batches = if cfg!(miri) { 4 } else { 2_000 };
    let size = 5u64;
    let total = producers * batches * size;
    let (ring, _mem) = make(total.next_power_of_two() as usize, mode);
    let mut cursor = ring.cursor_oldest();
    let handles: Vec<_> = (0..producers)
        .map(|p| {
            let ring = ring.clone();
            thread::spawn(move || {
                let mut publisher = ring.publisher();
                for b in 0..batches {
                    assert_eq!(
                        publisher
                            .publish_batch(&batch(p, b * size + 1, size))
                            .unwrap(),
                        size as usize
                    );
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let mut last = vec![0u64; producers as usize];
    let mut out = vec![0u8; MSG * 32];
    let mut count = 0u64;
    loop {
        let n = ring.read_batch(&mut cursor, &mut out).unwrap();
        if n == 0 {
            break;
        }
        for chunk in out.chunks_exact(MSG).take(n) {
            let (p, v) = decode(chunk);
            assert_eq!(v, last[p as usize] + 1, "producer {p} reordered");
            last[p as usize] = v;
            count += 1;
        }
    }
    assert_eq!(count, total);
}

#[test]
fn multi_producer_batches_keep_per_producer_order() {
    multi_run(LeanMode::MultiProducer);
}

#[test]
fn multi_producer_gated_batches_are_lossless() {
    let producers = 2u64;
    let batches = if cfg!(miri) { 4 } else { 5_000 };
    let size = 3u64;
    let (ring, _mem) = make(32, LeanMode::MultiProducerGated);
    let mut consumer = ring.register_consumer(true).unwrap();
    let handles: Vec<_> = (0..producers)
        .map(|p| {
            let ring = ring.clone();
            thread::spawn(move || {
                let mut publisher = ring.publisher();
                let mut next = 1u64;
                let end = batches * size + 1;
                while next < end {
                    let want = size.min(end - next);
                    match publisher.publish_batch(&batch(p, next, want)) {
                        Ok(n) => next += n as u64,
                        Err(_) => std::hint::spin_loop(),
                    }
                }
            })
        })
        .collect();
    let total = producers * batches * size;
    let mut last = vec![0u64; producers as usize];
    let mut out = vec![0u8; MSG * 8];
    let mut count = 0u64;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while count < total {
        assert!(std::time::Instant::now() < deadline, "starved at {count}");
        let n = ring.read_batch_gated(&mut consumer, &mut out).unwrap();
        for chunk in out.chunks_exact(MSG).take(n) {
            let (p, v) = decode(chunk);
            assert_eq!(v, last[p as usize] + 1, "producer {p} lost or reordered");
            last[p as usize] = v;
            count += 1;
        }
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(consumer.lost(), 0);
}
