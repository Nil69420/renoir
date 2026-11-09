use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use renoir::{
    topic::{Message, TopicStats},
    topic_rings::{MPMCTopicRing, SPSCTopicRing},
};
use std::sync::Arc;

fn benchmark_spsc_publish_consume(c: &mut Criterion) {
    let mut group = c.benchmark_group("TopicRing_SPSC_PublishConsume");

    for (label, capacity, payload_len) in [
        ("cap_4k_payload_64", 4096usize, 64usize),
        ("cap_16k_payload_256", 16384usize, 256usize),
        ("cap_64k_payload_1k", 65536usize, 1024usize),
    ] {
        group.throughput(Throughput::Elements(payload_len as u64));
        group.bench_with_input(
            BenchmarkId::new(label, payload_len),
            &(capacity, payload_len),
            |b, &(capacity, payload_len)| {
                let stats = Arc::new(TopicStats::default());
                let ring = SPSCTopicRing::new(capacity, stats).unwrap();
                let payload = vec![0xABu8; payload_len];

                b.iter(|| {
                    let message = Message::new_inline(1, 1, payload.clone());
                    ring.try_publish(&message).unwrap();
                    let consumed = ring.try_consume().unwrap().unwrap();
                    criterion::black_box(consumed);
                });
            },
        );
    }

    group.finish();
}

fn benchmark_mpmc_publish_consume(c: &mut Criterion) {
    let mut group = c.benchmark_group("TopicRing_MPMC_PublishConsume");

    for (label, capacity, payload_len) in [
        ("cap_4k_payload_64", 4096usize, 64usize),
        ("cap_16k_payload_256", 16384usize, 256usize),
    ] {
        group.throughput(Throughput::Elements(payload_len as u64));
        group.bench_with_input(
            BenchmarkId::new(label, payload_len),
            &(capacity, payload_len),
            |b, &(capacity, payload_len)| {
                let stats = Arc::new(TopicStats::default());
                let ring = MPMCTopicRing::new(capacity, stats).unwrap();
                let payload = vec![0xCDu8; payload_len];

                b.iter(|| {
                    let message = Message::new_inline(1, 1, payload.clone());
                    ring.try_publish(&message).unwrap();
                    let consumed = ring.try_consume().unwrap().unwrap();
                    criterion::black_box(consumed);
                });
            },
        );
    }

    group.finish();
}

fn benchmark_spsc_wraparound_stress(c: &mut Criterion) {
    let mut group = c.benchmark_group("TopicRing_SPSC_Wraparound");
    let capacity = 1024usize;

    group.bench_function("publish_consume_10k", |b| {
        let stats = Arc::new(TopicStats::default());
        let ring = SPSCTopicRing::new(capacity, stats).unwrap();
        let payload = vec![0x42u8; 32];

        b.iter(|| {
            for _ in 0..10_000 {
                let message = Message::new_inline(1, 1, payload.clone());
                ring.try_publish(&message).unwrap();
                let consumed = ring.try_consume().unwrap().unwrap();
                criterion::black_box(consumed);
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_spsc_publish_consume,
    benchmark_mpmc_publish_consume,
    benchmark_spsc_wraparound_stress
);
criterion_main!(benches);
