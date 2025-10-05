use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use renoir::ringbuf::{RingBuffer, SequencedRingBuffer};
use std::{sync::Arc, thread, time::Duration};

fn benchmark_single_threaded_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("RingBuffer_SingleThreaded");

    for capacity in [1024, 4096, 16384].iter() {
        group.throughput(Throughput::Elements(*capacity as u64));
        group.bench_with_input(
            BenchmarkId::new("push_pop_u64", capacity),
            capacity,
            |b, &capacity| {
                let buffer: RingBuffer<u64> = RingBuffer::new(capacity).unwrap();
                let producer = buffer.producer();
                let consumer = buffer.consumer();

                b.iter(|| {
                    // Fill buffer completely
                    for i in 0..capacity {
                        producer.try_push(i as u64).unwrap();
                    }

                    // Empty buffer completely
                    for _ in 0..capacity {
                        consumer.try_pop().unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

fn benchmark_different_data_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("RingBuffer_DataTypes");
    let capacity = 4096;

    // u8
    group.bench_function("u8", |b| {
        let buffer: RingBuffer<u8> = RingBuffer::new(capacity).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        b.iter(|| {
            for i in 0..255u8 {
                producer.try_push(i).unwrap();
            }
            for _ in 0..255 {
                consumer.try_pop().unwrap();
            }
        });
    });

    // u64
    group.bench_function("u64", |b| {
        let buffer: RingBuffer<u64> = RingBuffer::new(capacity).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        b.iter(|| {
            for i in 0..1000u64 {
                producer.try_push(i).unwrap();
            }
            for _ in 0..1000 {
                consumer.try_pop().unwrap();
            }
        });
    });

    // String (heap allocated)
    group.bench_function("String", |b| {
        let buffer: RingBuffer<String> = RingBuffer::new(capacity).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        b.iter(|| {
            for i in 0..100 {
                producer.try_push(format!("string_{}", i)).unwrap();
            }
            for _ in 0..100 {
                consumer.try_pop().unwrap();
            }
        });
    });

    // Fixed-size array
    group.bench_function("Array_64", |b| {
        let buffer: RingBuffer<[u8; 64]> = RingBuffer::new(capacity).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        b.iter(|| {
            for i in 0..500 {
                let mut arr = [0u8; 64];
                arr[0] = i as u8;
                producer.try_push(arr).unwrap();
            }
            for _ in 0..500 {
                consumer.try_pop().unwrap();
            }
        });
    });

    group.finish();
}

fn benchmark_sequenced_vs_regular(c: &mut Criterion) {
    let mut group = c.benchmark_group("RingBuffer_Comparison");
    let capacity = 4096;
    let iterations = 1000;

    group.bench_function("Regular_RingBuffer", |b| {
        let buffer: RingBuffer<u64> = RingBuffer::new(capacity).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        b.iter(|| {
            for i in 0..iterations {
                producer.try_push(i as u64).unwrap();
            }
            for _ in 0..iterations {
                consumer.try_pop().unwrap();
            }
        });
    });

    group.bench_function("Sequenced_RingBuffer", |b| {
        let buffer: SequencedRingBuffer<u64> = SequencedRingBuffer::new(capacity).unwrap();

        b.iter(|| {
            for i in 0..iterations {
                let guard = buffer.try_claim().unwrap();
                guard.write(i as u64);
            }
            for _ in 0..iterations {
                buffer.try_read().unwrap();
            }
        });
    });

    group.finish();
}

fn benchmark_contention_scenarios(c: &mut Criterion) {
    let mut group = c.benchmark_group("RingBuffer_Contention");
    let capacity = 8192;

    // High frequency, low contention
    group.bench_function("low_contention", |b| {
        let buffer = Arc::new(RingBuffer::<u64>::new(capacity).unwrap());

        b.iter(|| {
            let producer_buffer = Arc::clone(&buffer);
            let consumer_buffer = Arc::clone(&buffer);

            let producer_handle = thread::spawn(move || {
                let producer = producer_buffer.producer();
                for i in 0..5000u64 {
                    while producer.try_push(i).is_err() {
                        thread::yield_now();
                    }
                }
            });

            let consumer_handle = thread::spawn(move || {
                let consumer = consumer_buffer.consumer();
                let mut count = 0;
                while count < 5000 {
                    if consumer.try_pop().is_ok() {
                        count += 1;
                    } else {
                        thread::yield_now();
                    }
                }
            });

            producer_handle.join().unwrap();
            consumer_handle.join().unwrap();
        });
    });

    // Bursty traffic
    group.bench_function("bursty_traffic", |b| {
        let buffer = Arc::new(RingBuffer::<u64>::new(capacity).unwrap());

        b.iter(|| {
            let producer_buffer = Arc::clone(&buffer);
            let consumer_buffer = Arc::clone(&buffer);

            let producer_handle = thread::spawn(move || {
                let producer = producer_buffer.producer();

                // Burst 1
                for i in 0..1000u64 {
                    while producer.try_push(i).is_err() {
                        thread::yield_now();
                    }
                }

                thread::sleep(Duration::from_micros(10));

                // Burst 2
                for i in 1000..2000u64 {
                    while producer.try_push(i).is_err() {
                        thread::yield_now();
                    }
                }
            });

            let consumer_handle = thread::spawn(move || {
                let consumer = consumer_buffer.consumer();
                let mut count = 0;

                while count < 2000 {
                    if consumer.try_pop().is_ok() {
                        count += 1;
                    } else {
                        thread::yield_now();
                    }
                }
            });

            producer_handle.join().unwrap();
            consumer_handle.join().unwrap();
        });
    });

    group.finish();
}

fn benchmark_memory_access_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("RingBuffer_MemoryAccess");

    // Sequential access pattern
    group.bench_function("sequential_access", |b| {
        let buffer: RingBuffer<u64> = RingBuffer::new(4096).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        b.iter(|| {
            // Write sequentially
            for i in 0..2000u64 {
                producer.try_push(i).unwrap();
            }

            // Read sequentially
            for _ in 0..2000 {
                consumer.try_pop().unwrap();
            }
        });
    });

    // Random access pattern (simulated with varying intervals)
    group.bench_function("random_intervals", |b| {
        let buffer: RingBuffer<u64> = RingBuffer::new(4096).unwrap();
        let producer = buffer.producer();
        let consumer = buffer.consumer();

        b.iter(|| {
            let mut produced = 0;
            let mut consumed = 0;
            let target = 2000;

            while consumed < target {
                // Produce in bursts
                let produce_count = (produced + 50).min(target) - produced;
                for i in produced..produced + produce_count {
                    if producer.try_push(i as u64).is_err() {
                        break;
                    }
                    produced += 1;
                }

                // Consume in smaller chunks
                let consume_count = 20;
                for _ in 0..consume_count {
                    if consumer.try_pop().is_ok() {
                        consumed += 1;
                    } else {
                        break;
                    }
                }
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_single_threaded_throughput,
    benchmark_different_data_types,
    benchmark_sequenced_vs_regular,
    benchmark_contention_scenarios,
    benchmark_memory_access_patterns
);
criterion_main!(benches);
