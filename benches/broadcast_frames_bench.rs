use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use renoir::topic::{Message, MessageHeader, TopicStats, FORMAT_TYPE_INLINE};
use renoir::topic_rings::{BroadcastTopicRing, ReadInto};
use std::sync::Arc;

const FRAME: usize = 921_600;
const SLOTS: usize = 16;

fn make_ring() -> (Arc<BroadcastTopicRing>, Vec<u64>) {
    let ring_bytes = BroadcastTopicRing::slot_stride(FRAME) * SLOTS;
    let total = BroadcastTopicRing::required_memory_size(ring_bytes, FRAME);
    let mut memory = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(memory.as_mut_ptr() as *mut u8).unwrap();
    let ring = unsafe {
        BroadcastTopicRing::from_memory(
            ptr,
            memory.len() * 8,
            ring_bytes,
            FRAME,
            Arc::new(TopicStats::default()),
        )
        .unwrap()
    };
    (Arc::new(ring), memory)
}

fn header(seq: u64) -> MessageHeader {
    let mut h = MessageHeader::new(1, seq, FRAME as u32);
    h.set_format_type(FORMAT_TYPE_INLINE);
    h.set_no_checksum();
    h
}

fn bench_frames(c: &mut Criterion) {
    let mut group = c.benchmark_group("Broadcast_921600B_frame");
    group.throughput(Throughput::Bytes(FRAME as u64));
    group.sample_size(30);

    let (ring, _mem) = make_ring();
    let payload = vec![0x5Au8; FRAME];

    group.bench_function("publish_copy_with_crc", |b| {
        b.iter(|| {
            let m = Message::new_inline(1, 1, payload.clone());
            ring.try_publish(&m).unwrap();
        })
    });

    group.bench_function("publish_copy_no_crc", |b| {
        b.iter(|| {
            let m = Message::new_inline_unchecked(1, 1, payload.clone());
            ring.try_publish(&m).unwrap();
        })
    });

    group.bench_function("publish_reserve_commit_memset", |b| {
        let mut seq = 0u64;
        b.iter(|| {
            seq += 1;
            let r = ring.reserve(FRAME).unwrap();
            unsafe { std::ptr::write_bytes(r.payload_ptr(), 0x5A, FRAME) };
            ring.commit(r, &header(seq), FRAME).unwrap();
        })
    });

    group.bench_function("publish_reserve_commit_only", |b| {
        let mut seq = 0u64;
        b.iter(|| {
            seq += 1;
            let r = ring.reserve(FRAME).unwrap();
            ring.commit(r, &header(seq), FRAME).unwrap();
        })
    });

    let mut cursor = ring.cursor_oldest();
    let mut dst = vec![0u8; FRAME];
    group.bench_function("read_into_caller_buffer", |b| {
        b.iter(|| {
            let r = ring.reserve(FRAME).unwrap();
            ring.commit(r, &header(1), FRAME).unwrap();
            match ring.read_into(&mut cursor, &mut dst).unwrap() {
                ReadInto::Message { len, .. } => criterion::black_box(len),
                other => panic!("{:?}", other),
            };
        })
    });

    group.bench_function("try_consume_alloc_copy", |b| {
        b.iter(|| {
            let r = ring.reserve(FRAME).unwrap();
            ring.commit(r, &header(1), FRAME).unwrap();
            criterion::black_box(ring.try_consume(&mut cursor).unwrap().unwrap());
        })
    });

    group.finish();
}

criterion_group!(benches, bench_frames);
criterion_main!(benches);
