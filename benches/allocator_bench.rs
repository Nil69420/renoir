use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use renoir::{
    allocators::{Allocator, BumpAllocator, PoolAllocator},
    buffers::{BufferPool, BufferPoolConfig},
    memory::{BackingType, RegionConfig, SharedMemoryManager},
};
use std::{sync::Arc, time::Duration};

fn benchmark_bump_allocator(c: &mut Criterion) {
    let mut group = c.benchmark_group("BumpAllocator");

    for size in [64, 256, 1024, 4096].iter() {
        group.bench_with_input(BenchmarkId::new("allocate", size), size, |b, &size| {
            let mut memory = vec![0u8; 1024 * 1024]; // 1MB
            let allocator = BumpAllocator::new(&mut memory).unwrap();

            b.iter(|| {
                // Reset allocator for each iteration
                allocator.reset().unwrap();

                // Allocate many small chunks
                for _ in 0..100 {
                    let _ = allocator.allocate(size, 8);
                }
            });
        });
    }

    group.finish();
}

fn benchmark_pool_allocator(c: &mut Criterion) {
    let mut group = c.benchmark_group("PoolAllocator");

    for block_size in [64, 256, 1024, 4096].iter() {
        group.bench_with_input(
            BenchmarkId::new("allocate_deallocate", block_size),
            block_size,
            |b, &block_size| {
                let mut memory = vec![0u8; 1024 * 1024]; // 1MB
                let allocator = PoolAllocator::new(&mut memory, block_size).unwrap();

                b.iter(|| {
                    let mut ptrs = Vec::new();

                    // Allocate
                    for _ in 0..50 {
                        match allocator.allocate(block_size, 8) {
                            Ok(ptr) => ptrs.push(ptr),
                            Err(_) => break,
                        }
                    }

                    // Deallocate
                    for ptr in ptrs {
                        let _ = allocator.deallocate(ptr, block_size);
                    }
                });
            },
        );
    }

    group.finish();
}

fn benchmark_buffer_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("BufferPool");

    for buffer_size in [1024, 4096, 16384].iter() {
        group.bench_with_input(
            BenchmarkId::new("get_return", buffer_size),
            buffer_size,
            |b, &buffer_size| {
                let manager = SharedMemoryManager::new();
                let region_config = RegionConfig {
                    name: format!("bench_region_{}", buffer_size),
                    size: 64 * 1024 * 1024, // 64MB
                    backing_type: BackingType::FileBacked,
                    file_path: Some(std::path::PathBuf::from(format!(
                        "/tmp/renoir_bench_{}",
                        buffer_size
                    ))),
                    create: true,
                    permissions: 0o644,
                };

                let region = manager.create_region(region_config).unwrap();

                let pool_config = BufferPoolConfig {
                    name: "bench_pool".to_string(),
                    buffer_size,
                    initial_count: 100,
                    max_count: 500,
                    alignment: 64,
                    pre_allocate: true,
                    allocation_timeout: Some(Duration::from_millis(10)),
                };

                let buffer_pool = Arc::new(BufferPool::new(pool_config, region).unwrap());

                b.iter(|| {
                    let mut buffers = Vec::new();

                    // Get buffers
                    for _ in 0..20 {
                        match buffer_pool.get_buffer() {
                            Ok(buffer) => buffers.push(buffer),
                            Err(_) => break,
                        }
                    }

                    // Return buffers
                    for buffer in buffers {
                        let _ = buffer_pool.return_buffer(buffer);
                    }
                });
            },
        );
    }

    group.finish();
}

fn benchmark_ring_buffer(c: &mut Criterion) {
    use renoir::ringbuf::RingBuffer;

    let mut group = c.benchmark_group("RingBuffer");

    for capacity in [256, 1024, 4096].iter() {
        group.bench_with_input(
            BenchmarkId::new("push_pop", capacity),
            capacity,
            |b, &capacity| {
                let buffer: RingBuffer<u64> = RingBuffer::new(capacity).unwrap();
                let producer = buffer.producer();
                let consumer = buffer.consumer();

                b.iter(|| {
                    // Fill buffer
                    let mut count = 0;
                    while count < capacity / 2 {
                        if producer.try_push(count as u64).is_ok() {
                            count += 1;
                        } else {
                            break;
                        }
                    }

                    // Empty buffer
                    while consumer.try_pop().is_ok() {
                        // Continue popping
                    }
                });
            },
        );
    }

    group.finish();
}

fn benchmark_sequenced_ring_buffer(c: &mut Criterion) {
    use renoir::ringbuf::SequencedRingBuffer;

    let mut group = c.benchmark_group("SequencedRingBuffer");

    for capacity in [256, 1024, 4096].iter() {
        group.bench_with_input(
            BenchmarkId::new("claim_read", capacity),
            capacity,
            |b, &capacity| {
                let buffer: SequencedRingBuffer<u64> = SequencedRingBuffer::new(capacity).unwrap();

                b.iter(|| {
                    // Write some data
                    for i in 0..capacity / 4 {
                        if let Ok(guard) = buffer.try_claim() {
                            guard.write(i as u64);
                        }
                    }

                    // Read it back
                    while buffer.try_read().is_ok() {
                        // Continue reading
                    }
                });
            },
        );
    }

    group.finish();
}

fn benchmark_memory_copy(c: &mut Criterion) {
    let mut group = c.benchmark_group("MemoryCopy");

    for size in [1024, 4096, 16384, 65536].iter() {
        group.bench_with_input(BenchmarkId::new("memcpy", size), size, |b, &size| {
            let src = vec![0xABu8; size];
            let mut dst = vec![0u8; size];

            b.iter(|| {
                dst.copy_from_slice(&src);
            });
        });

        group.bench_with_input(
            BenchmarkId::new("buffer_write_read", size),
            size,
            |b, &size| {
                let manager = SharedMemoryManager::new();
                let region_config = RegionConfig {
                    name: format!("memcpy_region_{}", size),
                    size: 128 * 1024 * 1024, // 128MB
                    backing_type: BackingType::FileBacked,
                    file_path: Some(std::path::PathBuf::from(format!(
                        "/tmp/renoir_memcpy_{}",
                        size
                    ))),
                    create: true,
                    permissions: 0o644,
                };

                let region = manager.create_region(region_config).unwrap();

                let pool_config = BufferPoolConfig {
                    name: "memcpy_pool".to_string(),
                    buffer_size: size * 2, // Make sure buffer is large enough
                    initial_count: 10,
                    max_count: 20,
                    alignment: 64,
                    pre_allocate: true,
                    allocation_timeout: Some(Duration::from_millis(10)),
                };

                let buffer_pool = BufferPool::new(pool_config, region).unwrap();
                let src_data = vec![0xABu8; size];

                b.iter(|| {
                    let mut buffer = buffer_pool.get_buffer().unwrap();

                    // Write data
                    buffer.as_mut_slice()[..size].copy_from_slice(&src_data);
                    buffer.resize(size).unwrap();

                    // Read data back
                    let _read_data = buffer.as_slice().to_vec();

                    buffer_pool.return_buffer(buffer).unwrap();
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    benchmark_bump_allocator,
    benchmark_pool_allocator,
    benchmark_buffer_pool,
    benchmark_ring_buffer,
    benchmark_sequenced_ring_buffer,
    benchmark_memory_copy
);
criterion_main!(benches);
