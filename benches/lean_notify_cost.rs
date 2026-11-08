use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use renoir::topic_rings::{lean_slot_bytes, LeanMode, LeanRing};

fn ring(msg: usize, slots: usize, mode: LeanMode) -> (Arc<LeanRing>, Vec<u64>) {
    let ring_bytes = lean_slot_bytes(msg).unwrap() * slots;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let mut mem = vec![0u64; total / 8 + 8];
    let base = mem.as_mut_ptr() as usize;
    let aligned = (base + 63) & !63;
    let ptr = std::ptr::NonNull::new(aligned as *mut u8).unwrap();
    let ring = unsafe { LeanRing::from_memory_mode(ptr, total, ring_bytes, msg, mode).unwrap() };
    (Arc::new(ring), mem)
}

fn ring_mp() -> (Arc<LeanRing>, Vec<u64>) {
    ring(32, 1024, LeanMode::MultiProducer)
}

fn publish_cost(c: &mut Criterion) {
    let (ring, _mem) = ring(32, 1024, LeanMode::SingleProducer);
    let (mp_ring, _mp_mem) = ring_mp();
    let mut mp_publisher = mp_ring.publisher();
    let mut publisher = ring.publisher();
    let data = [7u8; 32];
    let mut group = c.benchmark_group("lean_publish_32B");
    group.bench_function("publish", |b| {
        b.iter(|| publisher.publish(black_box(&data)).unwrap())
    });
    group.bench_function("publish_notify_no_waiters", |b| {
        b.iter(|| publisher.publish_notify(black_box(&data)).unwrap())
    });
    group.bench_function("publish_multi_producer_uncontended", |b| {
        b.iter(|| mp_publisher.publish(black_box(&data)).unwrap())
    });
    let mut cursor = ring.cursor_latest();
    let mut sink = [0u8; 32];
    group.bench_function("publish_then_try_read", |b| {
        b.iter(|| {
            publisher.publish(black_box(&data)).unwrap();
            ring.try_read(&mut cursor, &mut sink).unwrap();
            black_box(&sink);
        })
    });
    let batch = [7u8; 32 * 8];
    group.bench_function("eight_singles", |b| {
        b.iter(|| {
            for chunk in batch.chunks_exact(32) {
                publisher.publish(black_box(chunk)).unwrap();
            }
        })
    });
    group.bench_function("batch_of_eight", |b| {
        b.iter(|| publisher.publish_batch(black_box(&batch)).unwrap())
    });
    group.bench_function("eight_singles_multi_producer", |b| {
        b.iter(|| {
            for chunk in batch.chunks_exact(32) {
                mp_publisher.publish(black_box(chunk)).unwrap();
            }
        })
    });
    group.bench_function("batch_of_eight_multi_producer", |b| {
        b.iter(|| mp_publisher.publish_batch(black_box(&batch)).unwrap())
    });
    group.finish();
}

criterion_group!(benches, publish_cost);
criterion_main!(benches);
