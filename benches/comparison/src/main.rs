//! Same-hardware, same-operation benchmark: Renoir SPSC topic ring vs
//! iceoryx2 publish-subscribe (SPSC topology — single publisher, single
//! subscriber, which is iceoryx2's default open-or-create service).
//!
//! Both sides: publish a fixed-size payload, receive it, repeat N times in
//! the same process (iceoryx2's ipc service is cross-process but works
//! same-process too; Renoir's ring is exercised via the same public API
//! path a real publisher/subscriber pair uses). Runs are executed
//! back-to-back in one invocation so background load is comparable.

use iceoryx2::prelude::*;
use std::hint::black_box;
use std::time::Instant;

const MSG_SIZE: usize = 256;
const ITERATIONS: usize = 100_000;

fn percentiles(mut latencies: Vec<u128>) -> (u128, u128, u128) {
    latencies.sort_unstable();
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[(latencies.len() * 99) / 100];
    let mean = latencies.iter().sum::<u128>() / latencies.len() as u128;
    (mean, p50, p99)
}

fn bench_renoir() -> (u128, u128, u128, f64) {
    use renoir::topic::{TopicConfig, TopicPattern};
    use renoir::topic_manager_modules::TopicManager;

    let manager = TopicManager::new().unwrap();
    let config = TopicConfig {
        name: "/bench/renoir".to_string(),
        pattern: TopicPattern::SPSC,
        ring_capacity: 1024,
        max_payload_size: MSG_SIZE * 2,
        ..Default::default()
    };
    manager.create_topic(config).unwrap();
    let publisher = manager.create_publisher("/bench/renoir").unwrap();
    let subscriber = manager.create_subscriber("/bench/renoir").unwrap();

    let payload = vec![0xABu8; MSG_SIZE];

    // Warmup
    for _ in 0..1000 {
        publisher.publish(payload.clone()).unwrap();
        subscriber.subscribe().unwrap().unwrap();
    }

    let mut latencies = Vec::with_capacity(ITERATIONS);
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let t0 = Instant::now();
        publisher.publish(payload.clone()).unwrap();
        let msg = subscriber.subscribe().unwrap().unwrap();
        let t1 = Instant::now();
        latencies.push(t1.duration_since(t0).as_nanos());
        black_box(msg);
    }
    let elapsed = start.elapsed().as_secs_f64();
    let (mean, p50, p99) = percentiles(latencies);
    let throughput = ITERATIONS as f64 / elapsed;
    (mean, p50, p99, throughput)
}

fn bench_iceoryx2() -> (u128, u128, u128, f64) {
    let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
    let service = node
        .service_builder(&"Bench/RenoirIox2".try_into().unwrap())
        .publish_subscribe::<[u8; MSG_SIZE]>()
        .open_or_create()
        .unwrap();
    let publisher = service.publisher_builder().create().unwrap();
    let subscriber = service.subscriber_builder().create().unwrap();

    let payload = [0xCDu8; MSG_SIZE];

    // Warmup
    for _ in 0..1000 {
        publisher.send_copy(payload).unwrap();
        while subscriber.receive().unwrap().is_none() {}
    }

    let mut latencies = Vec::with_capacity(ITERATIONS);
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let t0 = Instant::now();
        publisher.send_copy(payload).unwrap();
        loop {
            if let Some(sample) = subscriber.receive().unwrap() {
                let t1 = Instant::now();
                latencies.push(t1.duration_since(t0).as_nanos());
                black_box(sample);
                break;
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let (mean, p50, p99) = percentiles(latencies);
    let throughput = ITERATIONS as f64 / elapsed;
    (mean, p50, p99, throughput)
}

fn main() {
    let kernel = std::fs::read_to_string("/proc/version").unwrap_or_default();
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("model name"))
        .unwrap_or("unknown")
        .to_string();

    println!("=== Renoir vs iceoryx2 comparison ===");
    println!("Payload: {} bytes, iterations: {}", MSG_SIZE, ITERATIONS);
    println!("Hardware: {}", cpu.trim());
    println!("Kernel: {}", kernel.trim());
    println!("renoir: {}-{}", "0.1.0", env!("CARGO_PKG_VERSION"));
    println!("iceoryx2: 0.9.3");
    println!();

    // Interleave is not possible in a single process; run both in the same
    // invocation, back-to-back, and repeat the whole pair for stability.
    for round in 0..3 {
        let r = bench_renoir();
        let i = bench_iceoryx2();
        println!("Round {}:", round + 1);
        println!(
            "  renoir:   mean {:>7} ns | p50 {:>7} ns | p99 {:>7} ns | {:>8.0} msg/s",
            r.0, r.1, r.2, r.3
        );
        println!(
            "  iceoryx2: mean {:>7} ns | p50 {:>7} ns | p99 {:>7} ns | {:>8.0} msg/s",
            i.0, i.1, i.2, i.3
        );
    }
}
