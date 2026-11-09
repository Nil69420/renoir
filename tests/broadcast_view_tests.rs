use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;

use renoir::topic::{MessageHeader, TopicStats, FORMAT_TYPE_INLINE};
use renoir::topic_rings::{BroadcastCursor, BroadcastTopicRing, ViewRead};

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

fn publish(ring: &BroadcastTopicRing, seq: u64, len: usize, fill: u8) {
    let mut header = MessageHeader::new(1, seq, len as u32);
    header.set_format_type(FORMAT_TYPE_INLINE);
    header.set_no_checksum();
    let r = ring.reserve(len).unwrap();
    for i in 0..len {
        unsafe { r.payload_ptr().add(i).cast::<AtomicU8>().as_ref().unwrap() }
            .store(fill, Ordering::Relaxed);
    }
    ring.commit(r, &header, len).unwrap();
}

fn read_view_bytes(ptr: *const u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| unsafe { ptr.add(i).cast::<AtomicU8>().as_ref().unwrap() }.load(Ordering::Relaxed))
        .collect()
}

fn next_view(
    ring: &BroadcastTopicRing,
    cursor: &mut BroadcastCursor,
) -> Option<renoir::topic_rings::BroadcastView> {
    match ring.view(cursor).unwrap() {
        ViewRead::Empty => None,
        ViewRead::View(v) => Some(v),
    }
}

#[test]
fn view_exposes_payload_in_place_and_advances_on_release() {
    let (ring, _mem) = make_ring(8, 256);
    let mut cursor = ring.cursor_oldest();
    assert!(next_view(&ring, &mut cursor).is_none());
    publish(&ring, 1, 200, 0x11);
    publish(&ring, 2, 100, 0x22);
    let v = next_view(&ring, &mut cursor).unwrap();
    assert_eq!((v.len(), v.sequence()), (200, 1));
    assert_eq!(read_view_bytes(v.data_ptr(), v.len()), vec![0x11; 200]);
    assert!(ring.release_view(&mut cursor, &v).unwrap());
    let v = next_view(&ring, &mut cursor).unwrap();
    assert_eq!((v.len(), v.sequence()), (100, 2));
    assert!(ring.release_view(&mut cursor, &v).unwrap());
    assert!(next_view(&ring, &mut cursor).is_none());
    assert_eq!(cursor.overruns(), 0);
}

#[test]
fn view_overwritten_while_held_is_reported_not_intact() {
    let (ring, _mem) = make_ring(4, 64);
    let mut cursor = ring.cursor_oldest();
    publish(&ring, 1, 64, 0xAA);
    let v = next_view(&ring, &mut cursor).unwrap();
    for seq in 2..=6 {
        publish(&ring, seq, 64, 0xBB);
    }
    assert!(!ring.view_is_intact(&v));
    assert!(!ring.release_view(&mut cursor, &v).unwrap());
    assert_eq!(cursor.overruns(), 1);
}

#[test]
fn releasing_a_foreign_view_is_rejected() {
    let (ring, _mem) = make_ring(8, 64);
    let mut cursor = ring.cursor_oldest();
    publish(&ring, 1, 8, 1);
    publish(&ring, 2, 8, 2);
    let first = next_view(&ring, &mut cursor).unwrap();
    assert!(ring.release_view(&mut cursor, &first).unwrap());
    assert!(ring.release_view(&mut cursor, &first).is_err());
}

#[test]
fn intact_views_are_never_torn_under_concurrent_overwrite() {
    let frames: u64 = if cfg!(miri) { 40 } else { 200_000 };
    let (ring, _mem) = make_ring(4, 192);
    let mut cursor = ring.cursor_latest();
    let done = Arc::new(AtomicBool::new(false));
    let producer = {
        let ring = ring.clone();
        let done = done.clone();
        thread::spawn(move || {
            for seq in 1..=frames {
                publish(&ring, seq, 192, (seq % 251) as u8 + 1);
            }
            done.store(true, Ordering::Release);
        })
    };
    let mut intact = 0u64;
    let mut torn_but_flagged = 0u64;
    while !done.load(Ordering::Acquire) || next_peek(&ring, &mut cursor) {
        if let Some(v) = next_view(&ring, &mut cursor) {
            let bytes = read_view_bytes(v.data_ptr(), v.len());
            let ok = ring.release_view(&mut cursor, &v).unwrap();
            let uniform = bytes.iter().all(|b| *b == bytes[0]);
            if ok {
                assert!(uniform, "intact view was torn");
                assert_eq!(bytes[0], (v.sequence() % 251) as u8 + 1);
                intact += 1;
            } else if !uniform {
                torn_but_flagged += 1;
            }
        }
    }
    producer.join().unwrap();
    assert!(intact > 0);
    let _ = torn_but_flagged;
}

fn next_peek(ring: &BroadcastTopicRing, cursor: &mut BroadcastCursor) -> bool {
    let mut probe = *cursor;
    matches!(ring.view(&mut probe), Ok(ViewRead::View(_)))
}
