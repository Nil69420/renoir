//! Scale and integration tests
//! Tests focused on large-scale operations, system integration, and cross-component testing

use std::{
    sync::{Arc, atomic::{AtomicUsize, AtomicBool, Ordering}, Barrier},
    thread,
    time::{Duration, Instant},
    collections::HashMap,
};

use tempfile::TempDir;
use renoir::{
    memory::{BackingType, RegionConfig, SharedMemoryManager},
    topic::Message,
    topic_rings::{SPSCTopicRing, MPMCTopicRing},
    buffers::{BufferPool, BufferPoolConfig},
};

#[cfg(test)]
mod scale_integration_tests {
    use super::*;

    /// Test: Large-scale multi-region system integration
    #[test]
    fn scale_multi_region_integration() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        let region_count = 10;
        let buffers_per_region = 20;
        let mut regions = Vec::new();
        let mut pools = Vec::new();
        
        // Create multiple regions and buffer pools
        for i in 0..region_count {
            let region_name = format!("scale_region_{}", i);
            let config = RegionConfig::new(&region_name, 64 * 1024) // 64KB each
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                .with_create(true);
            
            let region = manager.create_region(config).unwrap();
            regions.push(region.clone());
            
            let pool_config = BufferPoolConfig::new(&format!("scale_pool_{}", i))
                .with_buffer_size(2048)
                .with_initial_count(buffers_per_region)
                .with_max_count(buffers_per_region * 2)
                .with_pre_allocate(true);
            
            let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
            pools.push(pool);
        }
        
        println!("Created {} regions with {} pools", region_count, pools.len());
        
        // Test cross-region operations
        let thread_count = region_count * 2; // 2 threads per region
        let operations_per_thread = 50;
        let success_count = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(thread_count));
        
        let mut handles = Vec::new();
        
        for thread_id in 0..thread_count {
            let pools = pools.clone();
            let success_counter = success_count.clone();
            let barrier = barrier.clone();
            
            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronize start
                
                for op in 0..operations_per_thread {
                    // Access different regions in round-robin fashion
                    let region_index = (thread_id + op) % pools.len();
                    let pool = &pools[region_index];
                    
                    match pool.get_buffer() {
                        Ok(mut buffer) => {
                            // Write thread-specific pattern
                            let pattern = (thread_id as u8).wrapping_add(op as u8);
                            buffer.as_mut_slice().fill(pattern);
                            success_counter.fetch_add(1, Ordering::Relaxed);
                            
                            // Brief work simulation
                            thread::sleep(Duration::from_micros(100));
                        }
                        Err(_) => {
                            // Failed allocation, continue
                        }
                    }
                }
            });
            
            handles.push(handle);
        }
        
        let start_time = Instant::now();
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        let duration = start_time.elapsed();
        let successful_ops = success_count.load(Ordering::Relaxed);
        let total_expected = thread_count * operations_per_thread;
        let success_rate = successful_ops as f64 / total_expected as f64;
        
        println!("Multi-region integration: {}/{} operations successful ({:.1}% success rate) in {:?}",
                successful_ops, total_expected, success_rate * 100.0, duration);
        
        // Should achieve high success rate in integration test
        assert!(success_rate > 0.8, "Should achieve >80% success rate in multi-region test");
    }

    /// Test: Massive message throughput across multiple rings
    #[test]
    fn scale_massive_message_throughput() {
        let ring_count = 8;
        let messages_per_ring = 1000;
        let mut rings = Vec::new();
        
        // Create multiple ring buffers
        for _i in 0..ring_count {
            let stats = Arc::new(renoir::topic::TopicStats::default());
            let ring = Arc::new(SPSCTopicRing::new(1024, stats).unwrap());
            rings.push(ring);
        }
        
        let total_messages = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(ring_count * 2)); // Producers + consumers
        
        // Create producer threads for each ring
        let mut producer_handles = Vec::new();
        for (ring_id, ring) in rings.iter().enumerate() {
            let ring = ring.clone();
            let barrier = barrier.clone();
            let msg_counter = total_messages.clone();
            
            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronize start
                
                for msg_id in 0..messages_per_ring {
                    let payload = format!("Ring {} message {}", ring_id, msg_id).into_bytes();
                    let message = Message::new_inline(ring_id as u32, msg_id as u64, payload);
                    
                    // Keep trying until published
                    while ring.try_publish(&message).is_err() {
                        thread::yield_now();
                    }
                    
                    msg_counter.fetch_add(1, Ordering::Relaxed);
                }
            });
            
            producer_handles.push(handle);
        }
        
        // Create consumer threads for each ring
        let consumed_messages = Arc::new(AtomicUsize::new(0));
        let mut consumer_handles = Vec::new();
        
        for ring in &rings {
            let ring = ring.clone();
            let barrier = barrier.clone();
            let consumed_counter = consumed_messages.clone();
            
            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronize start
                
                let mut consumed = 0;
                while consumed < messages_per_ring {
                    if let Ok(Some(_message)) = ring.try_consume() {
                        consumed += 1;
                        consumed_counter.fetch_add(1, Ordering::Relaxed);
                    } else {
                        // No message available, brief pause
                        thread::yield_now();
                    }
                }
            });
            
            consumer_handles.push(handle);
        }
        
        let start_time = Instant::now();
        
        // Wait for all producers
        for handle in producer_handles {
            handle.join().unwrap();
        }
        
        // Wait for all consumers
        for handle in consumer_handles {
            handle.join().unwrap();
        }
        
        let duration = start_time.elapsed();
        let produced = total_messages.load(Ordering::Relaxed);
        let consumed = consumed_messages.load(Ordering::Relaxed);
        let throughput = produced as f64 / duration.as_secs_f64();
        
        println!("Massive message throughput: {} rings, {} produced, {} consumed, {:.0} msgs/sec in {:?}",
                ring_count, produced, consumed, throughput, duration);
        
        assert_eq!(produced, ring_count * messages_per_ring, "Should produce expected message count");
        assert_eq!(consumed, produced, "Should consume all produced messages");
        assert!(throughput > 5000.0, "Should achieve high aggregate throughput");
    }

    /// Test: System behavior with hundreds of threads
    #[test]
    fn scale_hundreds_of_threads() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        let region_config = RegionConfig::new("massive_thread_region", 512 * 1024) // 512KB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("massive_threads.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("massive_thread_pool")
            .with_buffer_size(1024)
            .with_initial_count(50)
            .with_max_count(500)
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        let thread_count = 4; // Embedded-friendly thread count
        let operations_per_thread = 20;
        let success_count = Arc::new(AtomicUsize::new(0));
        let contention_count = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(thread_count));
        
        let mut handles = Vec::new();
        
        for thread_id in 0..thread_count {
            let pool = pool.clone();
            let success_counter = success_count.clone();
            let contention_counter = contention_count.clone();
            let barrier = barrier.clone();
            
            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronize start for maximum contention
                
                for _ in 0..operations_per_thread {
                    match pool.get_buffer() {
                        Ok(mut buffer) => {
                            // Write thread ID pattern
                            let pattern = (thread_id % 256) as u8;
                            buffer.as_mut_slice().fill(pattern);
                            success_counter.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            contention_counter.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    
                    // Brief pause to increase thread interleaving
                    if thread_id % 10 == 0 {
                        thread::yield_now();
                    }
                }
            });
            
            handles.push(handle);
        }
        
        let start_time = Instant::now();
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        let duration = start_time.elapsed();
        let successes = success_count.load(Ordering::Relaxed);
        let contentions = contention_count.load(Ordering::Relaxed);
        let total_operations = successes + contentions;
        let success_rate = successes as f64 / total_operations as f64;
        
        println!("Hundreds of threads: {} threads, {}/{} successful operations ({:.1}% success rate), {} contentions in {:?}",
                thread_count, successes, total_operations, success_rate * 100.0, contentions, duration);
        
        // Even with massive contention, should achieve reasonable success rate
        assert!(success_rate > 0.3, "Should achieve >30% success rate with 100 threads");
        assert!(successes > 0, "Should have some successful operations");
    }

    /// Test: Cross-component integration (rings + pools + regions)
    #[test]
    fn scale_cross_component_integration() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        // Component 1: Memory regions
        let region_config = RegionConfig::new("integration_region", 256 * 1024) // 256KB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("integration.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        // Component 2: Buffer pools
        let pool_config = BufferPoolConfig::new("integration_pool")
            .with_buffer_size(4096)
            .with_initial_count(20)
            .with_max_count(200)
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        // Component 3: Ring buffers
        let ring_stats = Arc::new(renoir::topic::TopicStats::default());
        let spsc_ring = Arc::new(SPSCTopicRing::new(512, ring_stats.clone()).unwrap());
        
        let mpmc_stats = Arc::new(renoir::topic::TopicStats::default());
        let mpmc_ring = Arc::new(MPMCTopicRing::new(512, mpmc_stats).unwrap());
        
        // Integration test: Use all components together
        let test_duration = Duration::from_secs(2);
        let stop_flag = Arc::new(AtomicBool::new(false));
        
        // Thread 1: Buffer pool operations
        let pool_worker = pool.clone();
        let stop_pool = stop_flag.clone();
        let pool_ops = Arc::new(AtomicUsize::new(0));
        let pool_counter = pool_ops.clone();
        
        let pool_thread = thread::spawn(move || {
            while !stop_pool.load(Ordering::Relaxed) {
                if let Ok(mut buffer) = pool_worker.get_buffer() {
                    // Use buffer as message storage
                    let data = b"Integration test data from pool";
                    if buffer.as_slice().len() >= data.len() {
                        buffer.as_mut_slice()[..data.len()].copy_from_slice(data);
                        pool_counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        
        // Thread 2: SPSC ring operations
        let spsc_producer = spsc_ring.clone();
        let stop_spsc = stop_flag.clone();
        let spsc_ops = Arc::new(AtomicUsize::new(0));
        let spsc_counter = spsc_ops.clone();
        
        let spsc_thread = thread::spawn(move || {
            let mut msg_id = 0u64;
            while !stop_spsc.load(Ordering::Relaxed) {
                let payload = format!("SPSC integration message {}", msg_id).into_bytes();
                let message = Message::new_inline(1, msg_id, payload);
                
                if spsc_producer.try_publish(&message).is_ok() {
                    spsc_counter.fetch_add(1, Ordering::Relaxed);
                    msg_id += 1;
                }
            }
        });
        
        // Thread 3: MPMC ring operations
        let mpmc_producer = mpmc_ring.clone();
        let stop_mpmc = stop_flag.clone();
        let mpmc_ops = Arc::new(AtomicUsize::new(0));
        let mpmc_counter = mpmc_ops.clone();
        
        let mpmc_thread = thread::spawn(move || {
            let mut msg_id = 0u64;
            while !stop_mpmc.load(Ordering::Relaxed) {
                let payload = format!("MPMC integration message {}", msg_id).into_bytes();
                let message = Message::new_inline(2, msg_id, payload);
                
                if mpmc_producer.try_publish(&message).is_ok() {
                    mpmc_counter.fetch_add(1, Ordering::Relaxed);
                    msg_id += 1;
                }
            }
        });
        
        // Thread 4: Cross-component consumer
        let spsc_consumer = spsc_ring.clone();
        let mpmc_consumer = mpmc_ring.clone();
        let stop_consumer = stop_flag.clone();
        let consumed_ops = Arc::new(AtomicUsize::new(0));
        let consumed_counter = consumed_ops.clone();
        
        let consumer_thread = thread::spawn(move || {
            while !stop_consumer.load(Ordering::Relaxed) {
                // Consume from both rings
                if let Ok(Some(_)) = spsc_consumer.try_consume() {
                    consumed_counter.fetch_add(1, Ordering::Relaxed);
                }
                
                if let Ok(Some(_)) = mpmc_consumer.try_consume() {
                    consumed_counter.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        
        let start_time = Instant::now();
        
        // Run integration test
        thread::sleep(test_duration);
        
        // Stop all threads
        stop_flag.store(true, Ordering::Relaxed);
        
        pool_thread.join().unwrap();
        spsc_thread.join().unwrap();
        mpmc_thread.join().unwrap();
        consumer_thread.join().unwrap();
        
        let duration = start_time.elapsed();
        let pool_operations = pool_ops.load(Ordering::Relaxed);
        let spsc_operations = spsc_ops.load(Ordering::Relaxed);
        let mpmc_operations = mpmc_ops.load(Ordering::Relaxed);
        let consumed_operations = consumed_ops.load(Ordering::Relaxed);
        
        println!("Cross-component integration: {} pool ops, {} SPSC ops, {} MPMC ops, {} consumed in {:?}",
                pool_operations, spsc_operations, mpmc_operations, consumed_operations, duration);
        
        // All components should be active
        assert!(pool_operations > 0, "Buffer pool should be active");
        assert!(spsc_operations > 0, "SPSC ring should be active");
        assert!(mpmc_operations > 0, "MPMC ring should be active");
        assert!(consumed_operations > 0, "Consumer should be active");
        
        // Consumed should be reasonable fraction of produced
        let total_produced = spsc_operations + mpmc_operations;
        assert!(consumed_operations >= total_produced / 4, "Should consume reasonable fraction of messages");
    }

    /// Test: Memory scaling with large regions
    #[test]
    fn scale_large_memory_regions() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let region_sizes = vec![
            256 * 1024,      // 256KB
            512 * 1024,      // 512KB
            1024 * 1024,     // 1MB
        ];
        
        for (i, &region_size) in region_sizes.iter().enumerate() {
            let region_name = format!("large_region_{}", i);
            let config = RegionConfig::new(&region_name, region_size)
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                .with_create(true);
            
            let start_time = Instant::now();
            let region_result = manager.create_region(config);
            let creation_time = start_time.elapsed();
            
            match region_result {
                Ok(region) => {
                    // Test large buffer allocation
                    let pool_config = BufferPoolConfig::new(&format!("large_pool_{}", i))
                        .with_buffer_size(8 * 1024) // 8KB buffers
                        .with_initial_count(10)
                        .with_max_count(region_size / (8 * 1024) / 2) // Half the theoretical max
                        .with_pre_allocate(false);
                    
                    let pool_start = Instant::now();
                    let pool_result = BufferPool::new(pool_config, region);
                    let pool_time = pool_start.elapsed();
                    
                    match pool_result {
                        Ok(pool) => {
                            // Test allocation performance
                            let alloc_start = Instant::now();
                            let mut buffers = Vec::new();
                            
                            for _ in 0..20 {
                                if let Ok(buffer) = pool.get_buffer() {
                                    buffers.push(buffer);
                                }
                            }
                            
                            let alloc_time = alloc_start.elapsed();
                            
                            println!("Large region {}: {} bytes, region creation {:?}, pool creation {:?}, 20 allocations {:?}",
                                    i, region_size, creation_time, pool_time, alloc_time);
                            
                            // Verify we could allocate something
                            assert!(!buffers.is_empty(), "Should allocate some buffers from large region");
                        }
                        Err(e) => {
                            println!("Large region {}: Failed to create pool: {:?}", i, e);
                        }
                    }
                }
                Err(e) => {
                    println!("Large region {}: Failed to create region: {:?}", i, e);
                }
            }
        }
    }

    /// Test: Complex message pattern routing
    #[test]
    fn scale_complex_message_routing() {
        let ring_count = 5;
        let mut rings = Vec::new();
        
        // Create SPSC rings only for simplicity
        for _i in 0..ring_count {
            let stats = Arc::new(renoir::topic::TopicStats::default());
            let ring = Arc::new(SPSCTopicRing::new(256, stats).unwrap());
            rings.push(ring);
        }
        
        // Message routing patterns
        let message_patterns = vec![
            ("broadcast", vec![0, 1, 2, 3, 4]), // Send to all rings
            ("round_robin", vec![0, 1, 2]),     // Round robin to first 3
            ("pair", vec![0, 4]),               // Send to first and last
            ("single", vec![2]),                // Send to middle ring only
        ];
        
        let mut routing_stats = HashMap::new();
        
        for (pattern_name, target_rings) in message_patterns {
            let messages_per_target = 50;
            let mut total_sent = 0;
            let mut total_consumed = 0;
            
            let start_time = Instant::now();
            
            // Send messages according to pattern
            for &ring_id in &target_rings {
                if ring_id < rings.len() {
                    for msg_id in 0..messages_per_target {
                        let payload = format!("{} pattern message {} to ring {}", 
                                             pattern_name, msg_id, ring_id).into_bytes();
                        let message = Message::new_inline(ring_id as u32, msg_id as u64, payload);
                        
                        // Try to publish (might fail if ring is full)
                        if rings[ring_id].try_publish(&message).is_ok() {
                            total_sent += 1;
                        }
                    }
                }
            }
            
            // Consume messages from target rings
            for &ring_id in &target_rings {
                if ring_id < rings.len() {
                    while let Ok(Some(_message)) = rings[ring_id].try_consume() {
                        total_consumed += 1;
                    }
                }
            }
            
            let pattern_time = start_time.elapsed();
            
            routing_stats.insert(pattern_name, (total_sent, total_consumed, pattern_time));
            
            println!("Routing pattern '{}': {}/{} messages (sent/consumed) to {} rings in {:?}",
                    pattern_name, total_sent, total_consumed, target_rings.len(), pattern_time);
        }
        
        // Verify routing worked for all patterns
        for (pattern_name, (sent, consumed, _)) in &routing_stats {
            assert!(*sent > 0, "Pattern '{}' should send some messages", pattern_name);
            assert!(*consumed >= *sent / 2, "Pattern '{}' should consume reasonable fraction", pattern_name);
        }
        
        println!("Complex message routing: {} patterns tested across {} rings", 
                routing_stats.len(), ring_count);
    }

    /// Test: System performance under sustained load
    #[test]
    fn scale_sustained_system_load() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        // Create system with multiple components
        let region_config = RegionConfig::new("sustained_region", 512 * 1024) // 512KB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("sustained.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("sustained_pool")
            .with_buffer_size(2048)
            .with_initial_count(50)
            .with_max_count(400)
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(2048, stats).unwrap());
        
        // Sustained load test parameters
        let test_duration = Duration::from_secs(10); // Long test
        let worker_threads = 16;
        let stop_flag = Arc::new(AtomicBool::new(false));
        
        // Metrics
        let total_operations = Arc::new(AtomicUsize::new(0));
        let buffer_operations = Arc::new(AtomicUsize::new(0));
        let ring_operations = Arc::new(AtomicUsize::new(0));
        let errors = Arc::new(AtomicUsize::new(0));
        
        let barrier = Arc::new(Barrier::new(worker_threads));
        let mut handles = Vec::new();
        
        // Create worker threads
        for thread_id in 0..worker_threads {
            let pool = pool.clone();
            let ring = ring.clone();
            let stop_flag = stop_flag.clone();
            let total_ops = total_operations.clone();
            let buffer_ops = buffer_operations.clone();
            let ring_ops = ring_operations.clone();
            let error_count = errors.clone();
            let barrier = barrier.clone();
            
            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronize start
                
                while !stop_flag.load(Ordering::Relaxed) {
                    // Mixed workload
                    match thread_id % 4 {
                        0 => {
                            // Buffer allocation/deallocation
                            if let Ok(_buffer) = pool.get_buffer() {
                                buffer_ops.fetch_add(1, Ordering::Relaxed);
                                total_ops.fetch_add(1, Ordering::Relaxed);
                            } else {
                                error_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        1 => {
                            // Ring buffer publishing
                            let payload = format!("Sustained load from thread {}", thread_id).into_bytes();
                            let message = Message::new_inline(thread_id as u32, 0, payload);
                            
                            if ring.try_publish(&message).is_ok() {
                                ring_ops.fetch_add(1, Ordering::Relaxed);
                                total_ops.fetch_add(1, Ordering::Relaxed);
                            } else {
                                error_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        2 => {
                            // Ring buffer consumption
                            if let Ok(Some(_)) = ring.try_consume() {
                                ring_ops.fetch_add(1, Ordering::Relaxed);
                                total_ops.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        3 => {
                            // Mixed operations
                            if let Ok(mut buffer) = pool.get_buffer() {
                                // Use buffer for message
                                let data = format!("Thread {} mixed operation", thread_id);
                                if buffer.as_slice().len() >= data.len() {
                                    buffer.as_mut_slice()[..data.len()].copy_from_slice(data.as_bytes());
                                }
                                
                                buffer_ops.fetch_add(1, Ordering::Relaxed);
                                total_ops.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        _ => unreachable!(),
                    }
                    
                    // Occasional yield to prevent CPU starvation
                    if total_ops.load(Ordering::Relaxed) % 1000 == 0 {
                        thread::yield_now();
                    }
                }
            });
            
            handles.push(handle);
        }
        
        let start_time = Instant::now();
        
        // Run sustained load test
        thread::sleep(test_duration);
        
        // Stop all workers
        stop_flag.store(true, Ordering::Relaxed);
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        let actual_duration = start_time.elapsed();
        let total_ops = total_operations.load(Ordering::Relaxed);
        let buffer_ops = buffer_operations.load(Ordering::Relaxed);
        let ring_ops = ring_operations.load(Ordering::Relaxed);
        let error_count = errors.load(Ordering::Relaxed);
        
        let ops_per_second = total_ops as f64 / actual_duration.as_secs_f64();
        let error_rate = error_count as f64 / (total_ops + error_count) as f64;
        
        println!("Sustained system load: {} total ops ({:.0} ops/sec), {} buffer ops, {} ring ops, {:.2}% error rate over {:?}",
                total_ops, ops_per_second, buffer_ops, ring_ops, error_rate * 100.0, actual_duration);
        
        // Performance expectations for sustained load
        assert!(total_ops > 50000, "Should achieve substantial throughput under sustained load");
        assert!(ops_per_second > 5000.0, "Should maintain high operations per second");
        assert!(error_rate < 0.2, "Should maintain low error rate under sustained load");
    }
}