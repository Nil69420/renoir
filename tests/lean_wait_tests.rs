use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use renoir::topic_rings::{lean_slot_bytes, LeanRead, LeanRing};

const MSG: usize = 16;

fn make(slots: usize) -> (Arc<LeanRing>, Vec<u64>) {
    let ring_bytes = lean_slot_bytes(MSG).unwrap() * slots;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring = unsafe { LeanRing::from_memory(ptr, mem.len() * 8, ring_bytes, MSG).unwrap() };
    (Arc::new(ring), mem)
}

fn payload(v: u64) -> [u8; MSG] {
    let mut b = [0u8; MSG];
    b[..8].copy_from_slice(&v.to_le_bytes());
    b[8..].copy_from_slice(&v.to_le_bytes());
    b
}

#[test]
fn wait_times_out_when_nothing_published() {
    let (ring, _mem) = make(16);
    let cursor = ring.cursor_latest();
    let start = Instant::now();
    assert!(!ring.wait_readable(&cursor, 100, Some(Duration::from_millis(50))));
    assert!(start.elapsed() >= Duration::from_millis(50));
}

#[test]
fn wait_returns_immediately_when_data_pending() {
    let (ring, _mem) = make(16);
    let cursor = ring.cursor_latest();
    ring.publisher().publish(&payload(1)).unwrap();
    assert!(ring.wait_readable(&cursor, 0, Some(Duration::ZERO)));
}

#[test]
fn huge_timeout_does_not_panic() {
    let (ring, _mem) = make(16);
    let cursor = ring.cursor_latest();
    ring.publisher().publish(&payload(1)).unwrap();
    assert!(ring.wait_readable(&cursor, 0, Some(Duration::MAX)));
}

#[test]
fn blocked_waiter_is_woken_by_publish_notify() {
    let (ring, _mem) = make(16);
    let mut cursor = ring.cursor_latest();
    let mut publisher = ring.publisher();
    let waiter = {
        let ring = ring.clone();
        thread::spawn(move || {
            let start = Instant::now();
            let ready = ring.wait_readable(&cursor, 0, Some(Duration::from_secs(10)));
            let mut out = [0u8; MSG];
            let read = ring.try_read(&mut cursor, &mut out).unwrap();
            (ready, read, start.elapsed(), out)
        })
    };
    thread::sleep(Duration::from_millis(100));
    publisher.publish_notify(&payload(7)).unwrap();
    let (ready, read, elapsed, out) = waiter.join().unwrap();
    assert!(ready);
    assert_eq!(read, LeanRead::Message);
    assert!(elapsed < Duration::from_secs(5));
    assert_eq!(out, payload(7));
}

#[test]
fn no_missed_wakeups_under_stress() {
    let count: u64 = if cfg!(miri) { 50 } else { 100_000 };
    let (ring, _mem) = make(if cfg!(miri) { 64 } else { 131_072 });
    let mut cursor = ring.cursor_oldest();
    let mut publisher = ring.publisher();
    let consumer = {
        let ring = ring.clone();
        thread::spawn(move || {
            let mut out = [0u8; MSG];
            let mut seen = 0u64;
            while seen < count {
                assert!(
                    ring.wait_readable(&cursor, 0, Some(Duration::from_secs(10))),
                    "consumer starved after {seen} messages"
                );
                while ring.try_read(&mut cursor, &mut out).unwrap() == LeanRead::Message {
                    seen += 1;
                    assert_eq!(out, payload(seen));
                }
            }
            seen
        })
    };
    for v in 1..=count {
        publisher.publish_notify(&payload(v)).unwrap();
    }
    assert_eq!(consumer.join().unwrap(), count);
}
