use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use renoir::topic_rings::{lean_slot_bytes, LeanCursor, LeanRead, LeanRing, LEAN_HEADER_BYTES};

fn make(slots: usize, msg: usize) -> (Arc<LeanRing>, Vec<u64>) {
    let slot_bytes = lean_slot_bytes(msg).unwrap();
    let ring_bytes = slot_bytes * slots;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring = unsafe { LeanRing::from_memory(ptr, mem.len() * 8, ring_bytes, msg).unwrap() };
    (Arc::new(ring), mem)
}

fn words(idx: u64, msg: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(msg);
    while v.len() < msg {
        v.extend_from_slice(&idx.to_ne_bytes());
    }
    v.truncate(msg);
    v
}

fn read_all(ring: &LeanRing, cursor: &mut LeanCursor, msg: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; msg];
    while let Ok(LeanRead::Message) = ring.try_read(cursor, &mut buf) {
        out.push(buf.clone());
    }
    out
}

#[test]
fn test_slot_size_rules() {
    assert_eq!(lean_slot_bytes(0), None);
    assert_eq!(lean_slot_bytes(1), Some(64));
    assert_eq!(lean_slot_bytes(56), Some(64));
    assert_eq!(lean_slot_bytes(57), Some(128));
    assert_eq!(lean_slot_bytes(504), Some(512));
    assert_eq!(lean_slot_bytes(505), None);
}

#[test]
fn test_one_cache_line_message_round_trips_in_order_for_two_subscribers() {
    let (ring, _mem) = make(16, 56);
    let mut publisher = ring.publisher();
    let mut a = ring.cursor_latest();
    let mut b = ring.cursor_latest();
    for i in 1..=10u64 {
        publisher.publish(&words(i, 56)).unwrap();
    }
    for cursor in [&mut a, &mut b] {
        let got = read_all(&ring, cursor, 56);
        assert_eq!(got.len(), 10);
        for (k, m) in got.iter().enumerate() {
            assert_eq!(m, &words(k as u64 + 1, 56));
        }
        assert_eq!(cursor.lost(), 0);
    }
}

#[test]
fn test_wrong_payload_or_buffer_size_is_rejected() {
    let (ring, _mem) = make(8, 24);
    let mut publisher = ring.publisher();
    assert!(publisher.publish(&[0u8; 23]).is_err());
    assert!(publisher.publish(&[0u8; 25]).is_err());
    publisher.publish(&[7u8; 24]).unwrap();
    let mut cursor = ring.cursor_oldest();
    let mut small = [0u8; 23];
    assert!(ring.try_read(&mut cursor, &mut small).is_err());
    let mut ok = [0u8; 24];
    assert_eq!(
        ring.try_read(&mut cursor, &mut ok).unwrap(),
        LeanRead::Message
    );
    assert_eq!(ok, [7u8; 24]);
}

#[test]
fn test_multi_line_messages_round_trip() {
    for msg in [57usize, 120, 200, 504] {
        let (ring, _mem) = make(8, msg);
        let mut publisher = ring.publisher();
        let mut cursor = ring.cursor_latest();
        publisher.publish(&words(5, msg)).unwrap();
        assert_eq!(read_all(&ring, &mut cursor, msg), vec![words(5, msg)]);
    }
}

#[test]
fn test_slow_reader_loses_messages_and_keeps_going() {
    let (ring, _mem) = make(4, 8);
    let mut publisher = ring.publisher();
    let mut cursor = ring.cursor_latest();
    for i in 1..=20u64 {
        publisher.publish(&words(i, 8)).unwrap();
    }
    let got = read_all(&ring, &mut cursor, 8);
    assert!(!got.is_empty());
    assert!(cursor.lost() > 0);
    let last = u64::from_ne_bytes(got.last().unwrap()[..8].try_into().unwrap());
    assert_eq!(last, 20);
    assert_eq!(got.len() as u64 + cursor.lost(), 20);
}

#[test]
fn test_late_joiner_and_oldest_cursor() {
    let (ring, _mem) = make(8, 8);
    let mut publisher = ring.publisher();
    for i in 1..=5u64 {
        publisher.publish(&words(i, 8)).unwrap();
    }
    let mut latest = ring.cursor_latest();
    let mut oldest = ring.cursor_oldest();
    assert!(read_all(&ring, &mut latest, 8).is_empty());
    assert_eq!(read_all(&ring, &mut oldest, 8).len(), 5);
    publisher.publish(&words(6, 8)).unwrap();
    assert_eq!(read_all(&ring, &mut latest, 8), vec![words(6, 8)]);
}

#[test]
fn test_publisher_resumes_after_reattach() {
    let (ring, _mem) = make(8, 8);
    let mut first = ring.publisher();
    first.publish(&words(1, 8)).unwrap();
    first.publish(&words(2, 8)).unwrap();
    let mut second = ring.publisher();
    second.publish(&words(3, 8)).unwrap();
    let mut cursor = ring.cursor_oldest();
    assert_eq!(
        read_all(&ring, &mut cursor, 8),
        vec![words(1, 8), words(2, 8), words(3, 8)]
    );
}

#[test]
fn test_skip_to_latest_and_pending() {
    let (ring, _mem) = make(16, 8);
    let mut publisher = ring.publisher();
    let mut cursor = ring.cursor_latest();
    for i in 1..=10u64 {
        publisher.publish(&words(i, 8)).unwrap();
    }
    assert_eq!(ring.pending(&cursor), 10);
    assert_eq!(ring.skip_to_latest(&mut cursor, 1), 9);
    assert_eq!(ring.pending(&cursor), 1);
    assert_eq!(read_all(&ring, &mut cursor, 8), vec![words(10, 8)]);
}

#[test]
fn test_geometry_mismatch_and_bad_sizes_rejected() {
    let (_ring, mut mem) = make(8, 24);
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring_bytes = lean_slot_bytes(24).unwrap() * 8;
    let other = unsafe { LeanRing::from_memory(ptr, mem.len() * 8, ring_bytes, 40) };
    assert!(other.is_err());
    let none = unsafe { LeanRing::from_memory(ptr, mem.len() * 8, ring_bytes, 0) };
    assert!(none.is_err());
    let too_small = unsafe { LeanRing::from_memory(ptr, 128, 1 << 20, 24) };
    assert!(too_small.is_err());
    const { assert!(LEAN_HEADER_BYTES.is_multiple_of(64)) };
}

#[test]
fn test_corrupt_future_slot_does_not_hang() {
    let (ring, mut mem) = make(8, 8);
    let mut publisher = ring.publisher();
    publisher.publish(&words(1, 8)).unwrap();
    unsafe {
        mem.as_mut_ptr()
            .add((LEAN_HEADER_BYTES + 64) / 8)
            .write(2 * 1000)
    };
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut cursor = ring.cursor_oldest();
        let mut buf = [0u8; 8];
        let first = ring.try_read(&mut cursor, &mut buf);
        let second = ring.try_read(&mut cursor, &mut buf);
        let _keep = mem;
        let _ = tx.send((first.is_ok(), second.is_ok()));
    });
    rx.recv_timeout(Duration::from_secs(10))
        .expect("reader hung");
}

#[test]
fn test_concurrent_first_attach_never_sees_half_built_header() {
    let rounds = if cfg!(miri) { 3 } else { 300 };
    let threads = 8usize;
    let msg = 16usize;
    let ring_bytes = lean_slot_bytes(msg).unwrap() * 8;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    for _ in 0..rounds {
        let mut memory = vec![0u64; total / 8 + 1];
        let base = memory.as_mut_ptr() as usize;
        let words_len = memory.len();
        let barrier = Arc::new(std::sync::Barrier::new(threads));
        let handles: Vec<_> = (0..threads)
            .map(|i| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
                    let ring = Arc::new(
                        unsafe { LeanRing::from_memory(ptr, words_len * 8, ring_bytes, msg) }
                            .expect("attach must succeed"),
                    );
                    if i == 0 {
                        ring.publisher().publish(&[1u8; 16]).unwrap();
                    }
                    ring.slot_count()
                })
            })
            .collect();
        let counts: Vec<usize> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(counts.iter().all(|c| *c == counts[0]));
        let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
        let ring = unsafe { LeanRing::from_memory(ptr, words_len * 8, ring_bytes, msg) }.unwrap();
        assert_eq!(ring.published(), 1);
        drop(memory);
    }
}

#[test]
fn test_concurrent_readers_never_see_torn_messages() {
    let total: u64 = if cfg!(miri) { 2_000 } else { 300_000 };
    let msg = 56usize;
    let (ring, _mem) = make(8, msg);
    let start = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let ring = ring.clone();
            let start = start.clone();
            let done = done.clone();
            let mut cursor = ring.cursor_latest();
            thread::spawn(move || {
                while !start.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
                let mut buf = [0u8; 56];
                let mut got = 0u64;
                let mut last = 0u64;
                loop {
                    match ring.try_read(&mut cursor, &mut buf) {
                        Ok(LeanRead::Message) => {
                            let first = u64::from_ne_bytes(buf[..8].try_into().unwrap());
                            assert!(first > last);
                            last = first;
                            for chunk in buf.chunks(8) {
                                assert_eq!(u64::from_ne_bytes(chunk.try_into().unwrap()), first);
                            }
                            got += 1;
                        }
                        Ok(LeanRead::Empty) => {
                            if done.load(Ordering::Acquire) && ring.pending(&cursor) == 0 {
                                break;
                            }
                            std::hint::spin_loop();
                        }
                        Err(_) => {}
                    }
                }
                (got, cursor.lost())
            })
        })
        .collect();
    let publisher = {
        let ring = ring.clone();
        let start = start.clone();
        let done = done.clone();
        thread::spawn(move || {
            let mut p = ring.publisher();
            start.store(true, Ordering::Release);
            for i in 1..=total {
                p.publish(&words(i, 56)).unwrap();
            }
            done.store(true, Ordering::Release);
        })
    };
    publisher.join().unwrap();
    for r in readers {
        let (got, lost) = r.join().unwrap();
        assert!(got > 0);
        assert_eq!(got + lost, total);
    }
}

#[test]
fn live_publishers_counts_registered_publishers_and_drops_on_release() {
    use renoir::topic_rings::LeanMode;
    for mode in [LeanMode::SingleProducer, LeanMode::MultiProducer] {
        let ring_bytes = lean_slot_bytes(16).unwrap() * 8;
        let total = LeanRing::required_memory_size(ring_bytes, 16);
        let mut mem = vec![0u64; total / 8 + 1];
        let ptr = std::ptr::NonNull::new(mem.as_mut_ptr().cast::<u8>()).unwrap();
        let ring = std::sync::Arc::new(unsafe {
            LeanRing::from_memory_mode(ptr, mem.len() * 8, ring_bytes, 16, mode).unwrap()
        });
        assert_eq!(ring.live_publishers(), 0);
        let mut first = ring.publisher();
        let mut second = ring.publisher();
        assert_eq!(ring.live_publishers(), 0);
        first.announce();
        second.announce();
        assert_eq!(ring.live_publishers(), 2);
        drop(first);
        assert_eq!(ring.live_publishers(), 1);
        drop(second);
        assert_eq!(ring.live_publishers(), 0);
    }
}
