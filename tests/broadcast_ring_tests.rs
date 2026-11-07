use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;

use renoir::topic::MessagePayload;
use renoir::topic::TopicStats;
use renoir::topic_rings::{BroadcastCursor, BroadcastTopicRing};
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

#[test]
fn test_broadcast_concurrent_no_torn_reads() {
    const TOTAL: u64 = 200_000;
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
