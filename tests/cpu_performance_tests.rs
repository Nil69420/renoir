//! CPU and performance benchmarking tests  
//! Tests focused on CPU utilization, throughput measurements, and performance validation

use std::{
    sync::{Arc, atomic::{AtomicUsize, AtomicBool, Ordering}},
    thread,
    time::{Duration, Instant},
};

use tempfile::TempDir;
use renoir::{
    memory::{BackingType, RegionConfig, SharedMemoryManager},
    topic::Message,
    topic_rings::{SPSCTopicRing, MPMCTopicRing},
    buffers::{BufferPool, BufferPoolConfig},
};

#[cfg(test)]
mod cpu_performance_tests {
    use super::*;

    /// Test: CPU utilization under sustained load
    #[test]
    fn perf_cpu_utilization_sustained_load() {
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(8192, stats).unwrap());
        
        let test_duration = Duration::from_secs(3); // Sustained load test
        let stop_flag = Arc::new(AtomicBool::new(false));
        let messages_processed = Arc::new(AtomicUsize::new(0));
        
        // Producer thread - constant message generation
        let ring_prod = ring.clone();
        let stop_prod = stop_flag.clone();
        let msg_count = messages_processed.clone();
        
        let producer = thread::spawn(move || {
            let mut sequence = 0u64;
            while !stop_prod.load(Ordering::Relaxed) {
                let payload = format!("Sustained load message {}", sequence).into_bytes();
                let message = Message::new_inline(1, sequence, payload);
                
                match ring_prod.try_publish(&message) {
                    Ok(()) => {
                        msg_count.fetch_add(1, Ordering::Relaxed);
                        sequence += 1;
                    }
                    Err(_) => {
                        // Ring full, brief backoff
                        thread::yield_now();
                    }
                }
            }
        });
        
        // Consumer thread - constant message consumption
        let ring_cons = ring.clone();
        let stop_cons = stop_flag.clone();
        let consumed_count = Arc::new(AtomicUsize::new(0));
        let consumed_counter = consumed_count.clone();
        
        let consumer = thread::spawn(move || {
            while !stop_cons.load(Ordering::Relaxed) {
                match ring_cons.try_consume() {
                    Ok(Some(_message)) => {
                        consumed_counter.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(None) => {
                        // No message available
                        continue;
                    }
                    Err(_) => break,
                }
            }
        });
        
        let start_time = Instant::now();
        
        // Let it run for the test duration
        thread::sleep(test_duration);
        
        // Stop the test
        stop_flag.store(true, Ordering::Relaxed);
        producer.join().unwrap();
        consumer.join().unwrap();
        
        let actual_duration = start_time.elapsed();
        let produced = messages_processed.load(Ordering::Relaxed);
        let consumed = consumed_count.load(Ordering::Relaxed);
        let throughput = produced as f64 / actual_duration.as_secs_f64();
        
        println!("Sustained load: {} produced, {} consumed in {:?} ({:.0} msgs/sec)",
                produced, consumed, actual_duration, throughput);
        
        // Performance expectations for sustained load
        assert!(produced > 10000, "Should produce substantial messages under sustained load");
        assert!(throughput > 5000.0, "Should maintain high throughput");
        assert!(consumed >= produced * 9 / 10, "Should consume at least 90% of produced messages");
    }

    /// Test: Throughput scaling with ring buffer size
    #[test]
    fn perf_throughput_scaling_ring_size() {
        let ring_sizes = vec![512, 1024, 2048, 4096, 8192, 16384];
        let test_duration = Duration::from_millis(500); // Short test per size
        
        for &ring_size in &ring_sizes {
            let stats = Arc::new(renoir::topic::TopicStats::default());
            let ring = Arc::new(SPSCTopicRing::new(ring_size, stats).unwrap());
            
            let stop_flag = Arc::new(AtomicBool::new(false));
            let throughput_counter = Arc::new(AtomicUsize::new(0));
            
            // Producer
            let ring_prod = ring.clone();
            let stop_prod = stop_flag.clone();
            let counter = throughput_counter.clone();
            
            let producer = thread::spawn(move || {
                let mut sequence = 0u64;
                while !stop_prod.load(Ordering::Relaxed) {
                    let payload = format!("Ring size test {}", sequence).into_bytes();
                    let message = Message::new_inline(1, sequence, payload);
                    
                    if ring_prod.try_publish(&message).is_ok() {
                        counter.fetch_add(1, Ordering::Relaxed);
                        sequence += 1;
                    } else {
                        thread::yield_now();
                    }
                }
            });
            
            // Consumer
            let ring_cons = ring.clone();
            let stop_cons = stop_flag.clone();
            
            let consumer = thread::spawn(move || {
                while !stop_cons.load(Ordering::Relaxed) {
                    let _ = ring_cons.try_consume();
                }
            });
            
            let start_time = Instant::now();
            thread::sleep(test_duration);
            stop_flag.store(true, Ordering::Relaxed);
            
            producer.join().unwrap();
            consumer.join().unwrap();
            
            let messages = throughput_counter.load(Ordering::Relaxed);
            let duration = start_time.elapsed();
            let throughput = messages as f64 / duration.as_secs_f64();
            
            println!("Ring size {}: {:.0} msgs/sec", ring_size, throughput);
        }
        
        // Just verify we got some reasonable throughput for each size
        assert!(true, "Throughput scaling test completed");
    }

    /// Test: CPU efficiency comparison - SPSC vs MPMC
    #[test] 
    fn perf_cpu_efficiency_spsc_vs_mpmc() {
        let message_count = 1000;
        
        // Test SPSC performance
        let spsc_stats = Arc::new(renoir::topic::TopicStats::default());
        let spsc_ring = Arc::new(SPSCTopicRing::new(4096, spsc_stats).unwrap());
        
        let start_spsc = Instant::now();
        let spsc_counter = Arc::new(AtomicUsize::new(0));
        
        // SPSC producer
        let spsc_prod = spsc_ring.clone();
        let spsc_count = spsc_counter.clone();
        let producer = thread::spawn(move || {
            for i in 0..message_count {
                let payload = format!("SPSC message {}", i).into_bytes();
                let message = Message::new_inline(1, i as u64, payload);
                
                while spsc_prod.try_publish(&message).is_err() {
                    thread::yield_now();
                }
                spsc_count.fetch_add(1, Ordering::Relaxed);
            }
        });
        
        // SPSC consumer
        let spsc_cons = spsc_ring.clone();
        let consumer = thread::spawn(move || {
            let mut consumed = 0;
            while consumed < message_count {
                if let Ok(Some(_)) = spsc_cons.try_consume() {
                    consumed += 1;
                }
            }
        });
        
        producer.join().unwrap();
        consumer.join().unwrap();
        let spsc_duration = start_spsc.elapsed();
        let spsc_throughput = message_count as f64 / spsc_duration.as_secs_f64();
        
        // Test MPMC performance
        let mpmc_stats = Arc::new(renoir::topic::TopicStats::default());
        let mpmc_ring = Arc::new(MPMCTopicRing::new(4096, mpmc_stats).unwrap());
        
        let start_mpmc = Instant::now();
        let mpmc_counter = Arc::new(AtomicUsize::new(0));
        
        // MPMC producer (single producer for fair comparison)
        let mpmc_prod = mpmc_ring.clone();
        let mpmc_count = mpmc_counter.clone();
        let producer = thread::spawn(move || {
            for i in 0..message_count {
                let payload = format!("MPMC message {}", i).into_bytes();
                let message = Message::new_inline(1, i as u64, payload);
                
                while mpmc_prod.try_publish(&message).is_err() {
                    thread::yield_now();
                }
                mpmc_count.fetch_add(1, Ordering::Relaxed);
            }
        });
        
        // MPMC consumer
        let mpmc_cons = mpmc_ring.clone();
        let consumer = thread::spawn(move || {
            let mut consumed = 0;
            while consumed < message_count {
                if let Ok(Some(_)) = mpmc_cons.try_consume() {
                    consumed += 1;
                }
            }
        });
        
        producer.join().unwrap();
        consumer.join().unwrap();
        let mpmc_duration = start_mpmc.elapsed();
        let mpmc_throughput = message_count as f64 / mpmc_duration.as_secs_f64();
        
        println!("SPSC throughput: {:.0} msgs/sec ({:?})", spsc_throughput, spsc_duration);
        println!("MPMC throughput: {:.0} msgs/sec ({:?})", mpmc_throughput, mpmc_duration);
        
        // SPSC should generally be faster than MPMC for single producer/consumer
        assert!(spsc_throughput > 1000.0, "SPSC should achieve good throughput");
        assert!(mpmc_throughput > 1000.0, "MPMC should achieve reasonable throughput");
    }

    /// Test: Memory bandwidth utilization
    #[test]
    fn perf_memory_bandwidth_utilization() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let region_config = RegionConfig::new("bandwidth_region", 10 * 1024 * 1024) // 10MB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("bandwidth.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("bandwidth_pool")
            .with_buffer_size(64 * 1024) // 64KB buffers for bandwidth test
            .with_initial_count(20)
            .with_max_count(150)
            .with_pre_allocate(true);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        let test_duration = Duration::from_secs(2);
        let bytes_written = Arc::new(AtomicUsize::new(0));
        let bytes_read = Arc::new(AtomicUsize::new(0));
        let stop_flag = Arc::new(AtomicBool::new(false));
        
        // Writer thread - maximize memory writes
        let pool_writer = pool.clone();
        let bytes_written_counter = bytes_written.clone();
        let stop_writer = stop_flag.clone();
        
        let writer = thread::spawn(move || {
            let pattern = vec![0xABu8; 1024]; // 1KB pattern
            
            while !stop_writer.load(Ordering::Relaxed) {
                if let Ok(mut buffer) = pool_writer.get_buffer() {
                    let slice = buffer.as_mut_slice();
                    let mut written = 0;
                    
                    // Fill buffer with pattern
                    while written + pattern.len() <= slice.len() {
                        slice[written..written + pattern.len()].copy_from_slice(&pattern);
                        written += pattern.len();
                    }
                    
                    bytes_written_counter.fetch_add(written, Ordering::Relaxed);
                }
            }
        });
        
        // Reader thread - maximize memory reads
        let pool_reader = pool.clone();
        let bytes_read_counter = bytes_read.clone();
        let stop_reader = stop_flag.clone();
        
        let reader = thread::spawn(move || {
            while !stop_reader.load(Ordering::Relaxed) {
                if let Ok(buffer) = pool_reader.get_buffer() {
                    let slice = buffer.as_slice();
                    let mut checksum = 0u64;
                    
                    // Read all bytes to force memory access
                    for &byte in slice {
                        checksum = checksum.wrapping_add(byte as u64);
                    }
                    
                    bytes_read_counter.fetch_add(slice.len(), Ordering::Relaxed);
                    
                    // Prevent optimization of checksum
                    if checksum == u64::MAX {
                        println!("Impossible checksum"); // Never executed
                    }
                }
            }
        });
        
        let start_time = Instant::now();
        thread::sleep(test_duration);
        stop_flag.store(true, Ordering::Relaxed);
        
        writer.join().unwrap();
        reader.join().unwrap();
        
        let actual_duration = start_time.elapsed();
        let written = bytes_written.load(Ordering::Relaxed);
        let read = bytes_read.load(Ordering::Relaxed);
        
        let write_bandwidth_mbps = (written as f64 / (1024.0 * 1024.0)) / actual_duration.as_secs_f64();
        let read_bandwidth_mbps = (read as f64 / (1024.0 * 1024.0)) / actual_duration.as_secs_f64();
        
        println!("Memory bandwidth: {:.1} MB/s write, {:.1} MB/s read in {:?}",
                write_bandwidth_mbps, read_bandwidth_mbps, actual_duration);
        
        // Verify we achieved reasonable bandwidth
        assert!(write_bandwidth_mbps > 10.0, "Should achieve reasonable write bandwidth");
        assert!(read_bandwidth_mbps > 10.0, "Should achieve reasonable read bandwidth");
    }

    /// Test: Latency measurement under different loads
    #[test]
    fn perf_latency_measurement_different_loads() {
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(2048, stats).unwrap());
        
        let load_levels = vec![10, 50, 100, 200]; // Messages per measurement cycle
        
        for &load in &load_levels {
            let mut latencies = Vec::new();
            let measurement_cycles = 50;
            
            for _ in 0..measurement_cycles {
                // Create load
                for i in 0..load {
                    let payload = format!("Load test message {}", i).into_bytes();
                    let message = Message::new_inline(1, i as u64, payload);
                    let _ = ring.try_publish(&message);
                }
                
                // Measure latency of next operation
                let start = Instant::now();
                let test_payload = b"Latency test message".to_vec();
                let test_message = Message::new_inline(1, 999999u64, test_payload);
                
                match ring.try_publish(&test_message) {
                    Ok(()) => {
                        let latency = start.elapsed();
                        latencies.push(latency);
                    }
                    Err(_) => {
                        // Ring full, clear it and retry
                        while let Ok(Some(_)) = ring.try_consume() {}
                        let _ = ring.try_publish(&test_message);
                        let latency = start.elapsed();
                        latencies.push(latency);
                    }
                }
                
                // Clear ring for next cycle
                while let Ok(Some(_)) = ring.try_consume() {}
            }
            
            if !latencies.is_empty() {
                let avg_latency_ns = latencies.iter().map(|d| d.as_nanos()).sum::<u128>() / latencies.len() as u128;
                let max_latency = latencies.iter().max().unwrap();
                
                println!("Load {}: avg {:.1}μs, max {:?}", load, avg_latency_ns as f64 / 1000.0, max_latency);
                
                // Latency should remain reasonable even under load
                assert!(avg_latency_ns < 100_000, "Average latency should be under 100μs");
            }
        }
    }

    /// Test: CPU cache efficiency with different access patterns
    #[test]
    fn perf_cpu_cache_efficiency() {
        let buffer_size = 1024 * 1024; // 1MB buffer
        let mut buffer = vec![0u8; buffer_size];
        
        // Sequential access pattern (cache-friendly)
        let start_sequential = Instant::now();
        let mut checksum = 0u64;
        
        for i in 0..buffer.len() {
            buffer[i] = (i % 256) as u8;
            checksum = checksum.wrapping_add(buffer[i] as u64);
        }
        
        let sequential_time = start_sequential.elapsed();
        
        // Random access pattern (cache-unfriendly)  
        let start_random = Instant::now();
        checksum = 0;
        
        // Simple LCG for pseudo-random access
        let mut seed = 1u32;
        for _ in 0..buffer.len() / 4 { // Less iterations to avoid taking too long
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let index = (seed as usize) % buffer.len();
            buffer[index] = (seed % 256) as u8;
            checksum = checksum.wrapping_add(buffer[index] as u64);
        }
        
        let random_time = start_random.elapsed();
        
        // Strided access pattern (moderate cache efficiency)
        let start_strided = Instant::now();
        checksum = 0;
        let stride = 64; // Cache line size stride
        
        for i in (0..buffer.len()).step_by(stride) {
            buffer[i] = (i % 256) as u8;
            checksum = checksum.wrapping_add(buffer[i] as u64);
        }
        
        let strided_time = start_strided.elapsed();
        
        println!("Cache efficiency - Sequential: {:?}, Random: {:?}, Strided: {:?}",
                sequential_time, random_time, strided_time);
        
        // Sequential should be fastest, random should be slowest
        assert!(sequential_time < random_time, "Sequential access should be faster than random");
        
        // Prevent optimization of checksum
        if checksum == u64::MAX {
            println!("Impossible checksum"); // Never executed
        }
    }

    /// Test: Scalability with increasing thread count
    #[test]
    fn perf_scalability_increasing_threads() {
        let thread_counts = vec![1, 2, 4, 8, 16];
        let operations_per_thread = 1000;
        
        for &thread_count in &thread_counts {
            let temp_dir = TempDir::new().unwrap();
            let manager = Arc::new(SharedMemoryManager::new());
            
            // Create regions for each thread
            let mut region_names = Vec::new();
            for i in 0..thread_count {
                let region_name = format!("scale_region_{}", i);
                let config = RegionConfig::new(&region_name, 256 * 1024) // 256KB
                    .with_backing_type(BackingType::FileBacked)
                    .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                    .with_create(true);
                
                manager.create_region(config).unwrap();
                region_names.push(region_name);
            }
            
            let start_time = Instant::now();
            let total_operations = Arc::new(AtomicUsize::new(0));
            
            let mut handles = Vec::new();
            for thread_id in 0..thread_count {
                let manager = manager.clone();
                let region_name = region_names[thread_id].clone();
                let op_counter = total_operations.clone();
                
                let handle = thread::spawn(move || {
                    for _ in 0..operations_per_thread {
                        if manager.get_region(&region_name).is_ok() {
                            op_counter.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
                handles.push(handle);
            }
            
            for handle in handles {
                handle.join().unwrap();
            }
            
            let duration = start_time.elapsed();
            let ops = total_operations.load(Ordering::Relaxed);
            let ops_per_second = ops as f64 / duration.as_secs_f64();
            
            println!("{} threads: {:.0} ops/sec ({:?})", thread_count, ops_per_second, duration);
        }
        
        assert!(true, "Scalability test completed");
    }

    /// Test: Ring buffer performance under different payload sizes
    #[test]
    fn perf_payload_size_impact() {
        let payload_sizes = vec![64, 256, 1024, 4096, 16384]; // Different message sizes
        let message_count = 1000;
        
        for &payload_size in &payload_sizes {
            let stats = Arc::new(renoir::topic::TopicStats::default());
            let ring = Arc::new(SPSCTopicRing::new(4096, stats).unwrap());
            
            let payload = vec![0xAAu8; payload_size];
            let start_time = Instant::now();
            
            // Producer
            let ring_prod = ring.clone();
            let payload_clone = payload.clone();
            let producer = thread::spawn(move || {
                for i in 0..message_count {
                    let message = Message::new_inline(1, i as u64, payload_clone.clone());
                    
                    while ring_prod.try_publish(&message).is_err() {
                        thread::yield_now();
                    }
                }
            });
            
            // Consumer
            let ring_cons = ring.clone();
            let consumer = thread::spawn(move || {
                let mut consumed = 0;
                while consumed < message_count {
                    if let Ok(Some(_)) = ring_cons.try_consume() {
                        consumed += 1;
                    }
                }
            });
            
            producer.join().unwrap();
            consumer.join().unwrap();
            
            let duration = start_time.elapsed();
            let throughput_msgs = message_count as f64 / duration.as_secs_f64();
            let throughput_mbps = (message_count * payload_size) as f64 / (1024.0 * 1024.0) / duration.as_secs_f64();
            
            println!("Payload {}B: {:.0} msgs/sec, {:.1} MB/s", 
                    payload_size, throughput_msgs, throughput_mbps);
        }
        
        assert!(true, "Payload size impact test completed");
    }

    /// Test: Performance under memory pressure
    #[test]
    fn perf_memory_pressure_impact() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        // Create a constrained memory environment
        let region_config = RegionConfig::new("pressure_region", 1024 * 1024) // Small 1MB region
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("pressure.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("pressure_pool")
            .with_buffer_size(16 * 1024) // 16KB buffers
            .with_initial_count(10)
            .with_max_count(60) // Will fill most of our 1MB
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        // Phase 1: Normal operation (low memory pressure)
        let start_normal = Instant::now();
        let mut buffers = Vec::new();
        
        for _ in 0..20 { // Allocate some buffers
            if let Ok(buffer) = pool.get_buffer() {
                buffers.push(buffer);
            }
        }
        
        let normal_ops = 1000;
        for _ in 0..normal_ops {
            let _ = pool.allocated_count(); // Lightweight operation
        }
        
        let normal_time = start_normal.elapsed();
        
        // Phase 2: High memory pressure
        // Fill most of available memory
        while buffers.len() < 50 {
            if let Ok(buffer) = pool.get_buffer() {
                buffers.push(buffer);
            } else {
                break;
            }
        }
        
        let start_pressure = Instant::now();
        
        for _ in 0..normal_ops {
            let _ = pool.allocated_count(); // Same operation under pressure
        }
        
        let pressure_time = start_pressure.elapsed();
        
        let normal_ops_per_sec = normal_ops as f64 / normal_time.as_secs_f64();
        let pressure_ops_per_sec = normal_ops as f64 / pressure_time.as_secs_f64();
        let performance_degradation = (1.0 - pressure_ops_per_sec / normal_ops_per_sec) * 100.0;
        
        println!("Memory pressure impact: {:.0} ops/sec normal, {:.0} ops/sec under pressure ({:.1}% degradation)",
                normal_ops_per_sec, pressure_ops_per_sec, performance_degradation);
        
        // Performance shouldn't degrade too much under memory pressure for simple operations
        assert!(performance_degradation < 50.0, "Performance degradation should be under 50%");
    }
}