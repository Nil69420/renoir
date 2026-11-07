use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use renoir::topic::MessagePayload;
use renoir::topic::TopicStats;
use renoir::topic::{MessageHeader, FORMAT_TYPE_INLINE};
use renoir::topic_rings::{BroadcastCursor, BroadcastTopicRing, ReadInto};
use renoir::Message;

fn make_ring(slots: usize, payload_cap: usize) -> (Arc<BroadcastTopicRing>, Vec<u64>) {
    let ring_bytes = BroadcastTopicRing::slot_stride(payload_cap) * slots;
    let total = BroadcastTopicRing::required_memory_size(ring_bytes, payload_cap);
    let mut memory = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(memory.as_mut_ptr() as *mut u8).unwrap();
    let ring = unsafe {
        BroadcastTopicRing::from_memory(
            ptr,
            memory.len() * 8,
            ring_bytes,
            payload_cap,
            Arc::new(TopicStats::default()),
        )
        .unwrap()
    };
    (Arc::new(ring), memory)
}

fn msg(seq: u64, len: usize) -> Message {
    Message::new_inline(1, seq, vec![(seq % 251) as u8; len])
}

fn payload_of(m: &Message) -> &Vec<u8> {
    match &m.payload {
        MessagePayload::Inline(d) => d,
        _ => panic!("expected inline payload"),
    }
}

fn seq_of(m: &Message) -> u64 {
    m.header.sequence
}

#[test]
fn test_geometry_matches_requested_slots() {
    let (ring, _mem) = make_ring(16, 1000);
    assert_eq!(ring.slot_count(), 16);
    assert_eq!(ring.payload_capacity(), 1000);
}

#[test]
fn test_two_subscribers_each_receive_every_message_in_order() {
    let (ring, _mem) = make_ring(16, 64);
    let mut a = ring.cursor_latest();
    let mut b = ring.cursor_latest();
    for i in 1..=10u64 {
        ring.try_publish(&msg(i, 8)).unwrap();
    }
    for cursor in [&mut a, &mut b] {
        for i in 1..=10u64 {
            let m = ring.try_consume(cursor).unwrap().unwrap();
            assert_eq!(seq_of(&m), i);
            assert_eq!(payload_of(&m), &vec![(i % 251) as u8; 8]);
        }
        assert!(ring.try_consume(cursor).unwrap().is_none());
        assert_eq!(cursor.overruns(), 0);
    }
}

#[test]
fn test_slow_subscriber_is_overrun_and_publisher_never_blocks() {
    let (ring, _mem) = make_ring(4, 64);
    let mut slow = ring.cursor_latest();
    for i in 1..=10u64 {
        ring.try_publish(&msg(i, 8)).unwrap();
    }
    let mut got = Vec::new();
    while let Some(m) = ring.try_consume(&mut slow).unwrap() {
        got.push(seq_of(&m));
    }
    assert_eq!(got, vec![8, 9, 10]);
    assert_eq!(slow.overruns(), 7);
}

#[test]
fn test_publish_with_no_subscribers_always_succeeds() {
    let (ring, _mem) = make_ring(4, 64);
    for i in 1..=1000u64 {
        ring.try_publish(&msg(i, 8)).unwrap();
    }
    assert_eq!(ring.published_count(), 1000);
}

#[test]
fn test_late_joiner_sees_only_new_messages_and_oldest_cursor_sees_history() {
    let (ring, _mem) = make_ring(8, 64);
    for i in 1..=5u64 {
        ring.try_publish(&msg(i, 8)).unwrap();
    }
    let mut latest = ring.cursor_latest();
    let mut oldest = ring.cursor_oldest();
    assert!(ring.try_consume(&mut latest).unwrap().is_none());
    ring.try_publish(&msg(6, 8)).unwrap();
    assert_eq!(seq_of(&ring.try_consume(&mut latest).unwrap().unwrap()), 6);
    let mut seen = Vec::new();
    while let Some(m) = ring.try_consume(&mut oldest).unwrap() {
        seen.push(seq_of(&m));
    }
    assert_eq!(seen, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn test_peek_does_not_advance_cursor() {
    let (ring, _mem) = make_ring(8, 64);
    let mut cursor = ring.cursor_latest();
    ring.try_publish(&msg(1, 8)).unwrap();
    let before = cursor.next_index();
    assert_eq!(seq_of(&ring.try_peek(&cursor).unwrap().unwrap()), 1);
    assert_eq!(seq_of(&ring.try_peek(&cursor).unwrap().unwrap()), 1);
    assert_eq!(cursor.next_index(), before);
    assert_eq!(seq_of(&ring.try_consume(&mut cursor).unwrap().unwrap()), 1);
}

#[test]
fn test_oversized_and_descriptor_payloads_are_rejected() {
    let (ring, _mem) = make_ring(4, 16);
    assert!(ring.try_publish(&msg(1, 17)).is_err());
    let desc = Message::new_descriptor(1, 1, renoir::topic::MessageDescriptor::new(0, 0, 0));
    assert!(ring.try_publish(&desc).is_err());
    assert_eq!(ring.published_count(), 0);
}

#[test]
fn test_zero_length_payload_round_trips() {
    let (ring, _mem) = make_ring(4, 16);
    let mut cursor = ring.cursor_latest();
    ring.try_publish(&msg(1, 0)).unwrap();
    let m = ring.try_consume(&mut cursor).unwrap().unwrap();
    assert!(payload_of(&m).is_empty());
}

#[test]
fn test_second_attach_shares_state_and_rejects_geometry_mismatch() {
    let (ring, mut mem) = make_ring(8, 64);
    ring.try_publish(&msg(1, 8)).unwrap();
    let ring_bytes = BroadcastTopicRing::slot_stride(64) * 8;
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let again = unsafe {
        BroadcastTopicRing::from_memory(
            ptr,
            mem.len() * 8,
            ring_bytes,
            64,
            Arc::new(TopicStats::default()),
        )
        .unwrap()
    };
    assert_eq!(again.published_count(), 1);
    let mut cursor = again.cursor_oldest();
    assert_eq!(seq_of(&again.try_consume(&mut cursor).unwrap().unwrap()), 1);

    let mismatch = unsafe {
        BroadcastTopicRing::from_memory(
            ptr,
            mem.len() * 8,
            ring_bytes,
            128,
            Arc::new(TopicStats::default()),
        )
    };
    assert!(mismatch.is_err());
}

#[test]
fn test_from_memory_rejects_undersized_and_too_few_slots() {
    let mut memory = vec![0u64; 16];
    let ptr = std::ptr::NonNull::new(memory.as_mut_ptr() as *mut u8).unwrap();
    let too_small = unsafe {
        BroadcastTopicRing::from_memory(ptr, 128, 1 << 20, 1024, Arc::new(TopicStats::default()))
    };
    assert!(too_small.is_err());
    let one_slot = unsafe {
        BroadcastTopicRing::from_memory(ptr, 128, 1024, 1024, Arc::new(TopicStats::default()))
    };
    assert!(one_slot.is_err());
}

fn zc_header(seq: u64, len: usize) -> MessageHeader {
    let mut h = MessageHeader::new(1, seq, len as u32);
    h.set_format_type(FORMAT_TYPE_INLINE);
    h.set_no_checksum();
    h
}

#[test]
fn test_reserved_payload_offsets_are_64_byte_aligned() {
    let (ring, mem) = make_ring(8, 1000);
    let base = mem.as_ptr() as usize;
    for i in 1..=20u64 {
        let r = ring.reserve(1000).unwrap();
        assert_eq!((r.payload_ptr() as usize - base) % 64, 0);
        ring.commit(r, &zc_header(i, 1000), 1000).unwrap();
    }
}

#[test]
fn test_reserve_commit_is_invisible_until_commit_and_readable_after() {
    let (ring, _mem) = make_ring(8, 128);
    let mut cursor = ring.cursor_latest();
    let r = ring.reserve(64).unwrap();
    unsafe { std::ptr::write_bytes(r.payload_ptr(), 0x42, 64) };
    assert!(ring.try_consume(&mut cursor).unwrap().is_none());
    assert!(ring.reserve(8).is_err());
    ring.commit(r, &zc_header(1, 64), 64).unwrap();
    let m = ring.try_consume(&mut cursor).unwrap().unwrap();
    assert_eq!(payload_of(&m), &vec![0x42u8; 64]);
    assert_eq!(ring.published_count(), 1);
}

#[test]
fn test_discarded_reservation_publishes_nothing_and_slot_is_reusable() {
    let (ring, _mem) = make_ring(8, 128);
    let mut cursor = ring.cursor_latest();
    let r = ring.reserve(64).unwrap();
    ring.discard(r);
    assert_eq!(ring.published_count(), 0);
    let r = ring.reserve(16).unwrap();
    unsafe { std::ptr::write_bytes(r.payload_ptr(), 0x07, 16) };
    ring.commit(r, &zc_header(1, 16), 16).unwrap();
    let m = ring.try_consume(&mut cursor).unwrap().unwrap();
    assert_eq!(payload_of(&m), &vec![0x07u8; 16]);
    assert!(ring.reserve(1000).is_err());
}

#[test]
fn test_read_into_copies_to_caller_buffer_and_reports_too_small_without_consuming() {
    let (ring, _mem) = make_ring(8, 128);
    let mut cursor = ring.cursor_latest();
    ring.try_publish(&msg(1, 40)).unwrap();
    let mut small = [0u8; 8];
    assert_eq!(
        ring.read_into(&mut cursor, &mut small).unwrap(),
        ReadInto::TooSmall(40)
    );
    let mut buf = [0u8; 128];
    match ring.read_into(&mut cursor, &mut buf).unwrap() {
        ReadInto::Message { len, sequence, .. } => {
            assert_eq!(len, 40);
            assert_eq!(sequence, 1);
            assert!(buf[..40].iter().all(|b| *b == 1));
        }
        other => panic!("unexpected {:?}", other),
    }
    assert_eq!(
        ring.read_into(&mut cursor, &mut buf).unwrap(),
        ReadInto::Empty
    );
}

#[test]
fn test_skip_to_latest_keeps_only_the_newest_unread_messages() {
    let (ring, _mem) = make_ring(16, 64);
    let mut cursor = ring.cursor_latest();
    for i in 1..=10u64 {
        ring.try_publish(&msg(i, 8)).unwrap();
    }
    assert_eq!(ring.skip_to_latest(&mut cursor, 1), 9);
    assert_eq!(cursor.skipped(), 9);
    assert_eq!(cursor.overruns(), 0);
    assert_eq!(seq_of(&ring.try_consume(&mut cursor).unwrap().unwrap()), 10);
    assert!(ring.try_consume(&mut cursor).unwrap().is_none());
    assert_eq!(ring.skip_to_latest(&mut cursor, 1), 0);

    for i in 11..=15u64 {
        ring.try_publish(&msg(i, 8)).unwrap();
    }
    assert_eq!(ring.skip_to_latest(&mut cursor, 3), 2);
    let mut got = Vec::new();
    while let Some(m) = ring.try_consume(&mut cursor).unwrap() {
        got.push(seq_of(&m));
    }
    assert_eq!(got, vec![13, 14, 15]);
    assert_eq!(ring.skip_to_latest(&mut cursor, 0), 0);
}

#[test]
fn test_wait_readable_times_out_when_nothing_is_published() {
    let (ring, _mem) = make_ring(8, 64);
    let cursor = ring.cursor_latest();
    let start = Instant::now();
    assert!(!ring.wait_readable(&cursor, Some(Duration::from_millis(60))));
    let waited = start.elapsed();
    assert!(
        waited >= Duration::from_millis(55),
        "returned early: {:?}",
        waited
    );
    assert!(
        waited < Duration::from_millis(500),
        "overslept: {:?}",
        waited
    );
}

#[test]
fn test_wait_readable_returns_immediately_when_message_pending() {
    let (ring, _mem) = make_ring(8, 64);
    let cursor = ring.cursor_latest();
    ring.try_publish(&msg(1, 8)).unwrap();
    let start = Instant::now();
    assert!(ring.wait_readable(&cursor, Some(Duration::from_secs(5))));
    assert!(start.elapsed() < Duration::from_millis(50));
}

#[test]
fn test_wait_readable_wakes_all_blocked_subscribers_on_publish() {
    let (ring, _mem) = make_ring(8, 64);
    let waiters: Vec<_> = (0..4)
        .map(|_| {
            let ring = ring.clone();
            let cursor = ring.cursor_latest();
            thread::spawn(move || {
                let start = Instant::now();
                let ok = ring.wait_readable(&cursor, Some(Duration::from_secs(10)));
                (ok, start.elapsed())
            })
        })
        .collect();
    thread::sleep(Duration::from_millis(150));
    ring.try_publish(&msg(1, 8)).unwrap();
    for w in waiters {
        let (ok, elapsed) = w.join().unwrap();
        assert!(ok);
        assert!(elapsed < Duration::from_secs(2), "woke late: {:?}", elapsed);
    }
}

#[test]
fn test_wake_latency_is_far_below_polling_interval() {
    let (ring, _mem) = make_ring(8, 64);
    let iterations = if cfg!(miri) { 20 } else { 2000 };
    let mut cursor = ring.cursor_latest();
    let published_at = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let epoch = Instant::now();

    let publisher = {
        let ring = ring.clone();
        let published_at = published_at.clone();
        thread::spawn(move || {
            for i in 1..=iterations as u64 {
                thread::sleep(Duration::from_micros(300));
                published_at.store(epoch.elapsed().as_nanos() as u64, Ordering::Release);
                ring.try_publish(&msg(i, 8)).unwrap();
            }
        })
    };

    let mut latencies = Vec::new();
    while latencies.len() < iterations {
        if ring.wait_readable(&cursor, Some(Duration::from_secs(5))) {
            let now = epoch.elapsed().as_nanos() as u64;
            let t = published_at.load(Ordering::Acquire);
            if let Some(m) = ring.try_consume(&mut cursor).unwrap() {
                let _ = m;
                latencies.push(now.saturating_sub(t));
            }
        } else {
            panic!("timed out waiting for publish");
        }
    }
    publisher.join().unwrap();
    latencies.sort_unstable();
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[latencies.len() * 99 / 100];
    eprintln!(
        "wake latency ns: p50={} p99={} max={}",
        p50,
        p99,
        latencies[latencies.len() - 1]
    );
    if !cfg!(miri) {
        assert!(p99 < 5_000_000, "p99 wake latency {} ns", p99);
    }
}

#[test]
fn test_concurrent_first_attach_never_sees_or_resets_a_half_built_header() {
    let rounds = if cfg!(miri) { 3 } else { 300 };
    let threads = 8usize;
    let payload_cap = 64usize;
    let ring_bytes = BroadcastTopicRing::slot_stride(payload_cap) * 8;
    let total = BroadcastTopicRing::required_memory_size(ring_bytes, payload_cap);
    for _ in 0..rounds {
        let mut memory = vec![0u64; total / 8 + 1];
        let base = memory.as_mut_ptr() as usize;
        let words = memory.len();
        let barrier = Arc::new(std::sync::Barrier::new(threads));
        let handles: Vec<_> = (0..threads)
            .map(|i| {
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
                    let ring = unsafe {
                        BroadcastTopicRing::from_memory(
                            ptr,
                            words * 8,
                            ring_bytes,
                            payload_cap,
                            Arc::new(TopicStats::default()),
                        )
                    }
                    .expect("attach must succeed");
                    if i == 0 {
                        ring.try_publish(&msg(1, 8)).unwrap();
                    }
                    ring.slot_count()
                })
            })
            .collect();
        let counts: Vec<usize> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(counts.iter().all(|c| *c == counts[0]));
        let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
        let ring = unsafe {
            BroadcastTopicRing::from_memory(
                ptr,
                words * 8,
                ring_bytes,
                payload_cap,
                Arc::new(TopicStats::default()),
            )
        }
        .unwrap();
        assert_eq!(ring.published_count(), 1);
        drop(memory);
    }
}

#[test]
fn test_broadcast_concurrent_no_torn_reads() {
    const TOTAL: u64 = if cfg!(miri) { 3_000 } else { 200_000 };
    let (ring, _mem) = make_ring(8, 256);
    let start = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));

    let readers: Vec<_> = (0..3)
        .map(|_| {
            let ring = ring.clone();
            let start = start.clone();
            let done = done.clone();
            let mut cursor: BroadcastCursor = ring.cursor_latest();
            thread::spawn(move || {
                while !start.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
                let mut received = 0u64;
                let mut last = 0u64;
                loop {
                    match ring.try_consume(&mut cursor).unwrap() {
                        Some(m) => {
                            let seq = seq_of(&m);
                            assert!(seq > last);
                            last = seq;
                            let data = payload_of(&m);
                            assert_eq!(data.len(), 1 + (seq % 200) as usize);
                            assert!(data.iter().all(|b| *b == (seq % 251) as u8));
                            received += 1;
                        }
                        None => {
                            if done.load(Ordering::Acquire) {
                                if ring.try_consume(&mut cursor).unwrap().is_none() {
                                    break;
                                }
                            } else {
                                std::hint::spin_loop();
                            }
                        }
                    }
                }
                (received, cursor.overruns())
            })
        })
        .collect();

    let publisher = {
        let ring = ring.clone();
        let start = start.clone();
        let done = done.clone();
        thread::spawn(move || {
            start.store(true, Ordering::Release);
            for i in 1..=TOTAL {
                ring.try_publish(&msg(i, 1 + (i % 200) as usize)).unwrap();
            }
            done.store(true, Ordering::Release);
        })
    };
    publisher.join().unwrap();

    for reader in readers {
        let (received, overruns) = reader.join().unwrap();
        assert!(received > 0);
        assert_eq!(received + overruns, TOTAL);
    }
}
