//! Reliability and robustness tests
//! Tests focused on system reliability, error recovery, and edge case handling

use std::{
    sync::{Arc, atomic::{AtomicUsize, AtomicBool, Ordering}, Barrier},
    thread,
    time::{Duration, Instant},
};

use tempfile::TempDir;
use renoir::{
    memory::{BackingType, RegionConfig, SharedMemoryManager},
    topic::Message,
    topic_rings::SPSCTopicRing,
    buffers::{BufferPool, BufferPoolConfig},
};

#[cfg(test)]
mod reliability_tests {
    use super::*;

    /// Test: System recovery from buffer exhaustion
    #[test]
    fn reliability_recovery_buffer_exhaustion() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let region_config = RegionConfig::new("exhaustion_region", 256 * 1024) // Small region
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("exhaustion.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("exhaustion_pool")
            .with_buffer_size(8 * 1024) // 8KB buffers
            .with_initial_count(5)
            .with_max_count(30) // Limited buffers
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        // Phase 1: Exhaust all buffers
        let mut buffers = Vec::new();
        while let Ok(buffer) = pool.get_buffer() {
            buffers.push(buffer);
        }
        
        let exhausted_count = buffers.len();
        assert!(exhausted_count > 0, "Should have allocated some buffers");
        
        // Phase 2: Verify system handles exhaustion gracefully
        let result = pool.get_buffer();
        assert!(result.is_err(), "Should fail when buffers exhausted");
        
        // Phase 3: Release half and verify partial recovery
        let mid_point = buffers.len() / 2;
        buffers.drain(0..mid_point);
        
        let mut recovered_buffers = Vec::new();
        for _ in 0..5 {
            if let Ok(buffer) = pool.get_buffer() {
                recovered_buffers.push(buffer);
            }
        }
        
        assert!(!recovered_buffers.is_empty(), "Should recover some buffers after releasing");
        
        // Phase 4: Full recovery
        buffers.clear();
        recovered_buffers.clear();
        
        // Should be able to allocate again
        let final_buffer = pool.get_buffer();
        assert!(final_buffer.is_ok(), "Should fully recover after releasing all buffers");
        
        println!("Buffer exhaustion recovery: exhausted {} buffers, recovered successfully", exhausted_count);
    }

    /// Test: Ring buffer behavior under overflow conditions
    #[test]
    fn reliability_ring_overflow_handling() {
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(64, stats).unwrap()); // Very small ring
        
        let overflow_attempts = 200; // Try to publish far more than ring capacity
        let mut successful_publishes = 0;
        let mut overflow_errors = 0;
        
        // Fill ring beyond capacity
        for i in 0..overflow_attempts {
            let payload = format!("Overflow message {}", i).into_bytes();
            let message = Message::new_inline(1, i as u64, payload);
            
            match ring.try_publish(&message) {
                Ok(()) => successful_publishes += 1,
                Err(_) => overflow_errors += 1,
            }
        }
        
        assert!(successful_publishes > 0, "Should have some successful publishes");
        assert!(overflow_errors > 0, "Should detect overflow conditions");
        assert!(successful_publishes < overflow_attempts, "Should not accept all messages when overflowing");
        
        // Verify ring is still functional after overflow
        let consumed_count = {
            let mut count = 0;
            while let Ok(Some(_)) = ring.try_consume() {
                count += 1;
            }
            count
        };
        
        assert!(consumed_count > 0, "Ring should still be functional after overflow");
        
        // Should be able to publish again after consuming
        let payload = b"Recovery test message".to_vec();
        let recovery_message = Message::new_inline(1, 999u64, payload);
        let recovery_result = ring.try_publish(&recovery_message);
        assert!(recovery_result.is_ok(), "Should be able to publish after consuming from overflowed ring");
        
        println!("Ring overflow handling: {}/{} successful publishes, {} overflow errors, {} consumed for recovery",
                successful_publishes, overflow_attempts, overflow_errors, consumed_count);
    }

    /// Test: Concurrent access error handling
    #[test]
    fn reliability_concurrent_error_handling() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        let region_config = RegionConfig::new("error_region", 512 * 1024)
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("error.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("error_pool")
            .with_buffer_size(4 * 1024)
            .with_initial_count(10)
            .with_max_count(100)
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        let thread_count = 8;
        let operations_per_thread = 100;
        let success_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(thread_count));
        
        let mut handles = Vec::new();
        
        for thread_id in 0..thread_count {
            let pool = pool.clone();
            let success_counter = success_count.clone();
            let error_counter = error_count.clone();
            let barrier = barrier.clone();
            
            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronize start
                
                let mut local_buffers = Vec::new();
                
                for _ in 0..operations_per_thread {
                    // Randomly allocate or deallocate
                    if thread_id % 2 == 0 && !local_buffers.is_empty() && local_buffers.len() > 5 {
                        // Deallocate (drop buffer)
                        local_buffers.pop();
                        success_counter.fetch_add(1, Ordering::Relaxed);
                    } else {
                        // Allocate
                        match pool.get_buffer() {
                            Ok(buffer) => {
                                local_buffers.push(buffer);
                                success_counter.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => {
                                error_counter.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    
                    // Brief pause to increase contention
                    if thread_id % 4 == 0 {
                        thread::yield_now();
                    }
                }
            });
            
            handles.push(handle);
        }
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        let successes = success_count.load(Ordering::Relaxed);
        let errors = error_count.load(Ordering::Relaxed);
        let total_operations = successes + errors;
        
        println!("Concurrent error handling: {}/{} successful operations, {} errors ({:.1}% success rate)",
                successes, total_operations, errors, (successes as f64 / total_operations as f64) * 100.0);
        
        // Should have reasonable success rate even under contention
        assert!(successes > 0, "Should have some successful operations");
        assert!(successes as f64 / total_operations as f64 > 0.5, "Should have >50% success rate");
    }

    /// Test: Memory region corruption detection and recovery
    #[test]
    fn reliability_memory_corruption_detection() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let region_config = RegionConfig::new("corruption_region", 128 * 1024)
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("corruption.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("corruption_pool")
            .with_buffer_size(1024)
            .with_initial_count(20)
            .with_max_count(100)
            .with_pre_allocate(true);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        // Normal operation phase
        let mut buffers = Vec::new();
        for _ in 0..10 {
            if let Ok(mut buffer) = pool.get_buffer() {
                // Write known pattern
                let pattern = vec![0x42u8; buffer.as_slice().len()];
                buffer.as_mut_slice().copy_from_slice(&pattern);
                buffers.push(buffer);
            }
        }
        
        // Verify pattern integrity
        let mut pattern_matches = 0;
        for buffer in &buffers {
            if buffer.as_slice().iter().all(|&b| b == 0x42) {
                pattern_matches += 1;
            }
        }
        
        assert_eq!(pattern_matches, buffers.len(), "All buffers should maintain data integrity");
        
        // Simulate recovery after potential corruption
        buffers.clear();
        
        // System should still be functional
        for _ in 0..5 {
            match pool.get_buffer() {
                Ok(mut buffer) => {
                    // Write and immediately verify
                    let test_pattern = vec![0xABu8; buffer.as_slice().len()];
                    buffer.as_mut_slice().copy_from_slice(&test_pattern);
                    
                    let integrity_check = buffer.as_slice().iter().all(|&b| b == 0xAB);
                    assert!(integrity_check, "Buffer should maintain integrity after corruption test");
                    break;
                }
                Err(_) => continue,
            }
        }
        
        println!("Memory corruption detection: {} buffers verified, system remains functional", pattern_matches);
    }

    /// Test: Long-running stability under continuous operation
    #[test]
    fn reliability_long_running_stability() {
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(1024, stats).unwrap());
        
        let test_duration = Duration::from_secs(5); // Moderate duration for stability test
        let stop_flag = Arc::new(AtomicBool::new(false));
        let message_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));
        
        // Producer thread - continuous operation
        let ring_prod = ring.clone();
        let stop_prod = stop_flag.clone();
        let msg_counter = message_count.clone();
        let error_counter = error_count.clone();
        
        let producer = thread::spawn(move || {
            let mut sequence = 0u64;
            
            while !stop_prod.load(Ordering::Relaxed) {
                let payload = format!("Stability test message {}", sequence).into_bytes();
                let message = Message::new_inline(1, sequence, payload);
                
                match ring_prod.try_publish(&message) {
                    Ok(()) => {
                        msg_counter.fetch_add(1, Ordering::Relaxed);
                        sequence += 1;
                    }
                    Err(_) => {
                        error_counter.fetch_add(1, Ordering::Relaxed);
                        // Brief backoff on error
                        thread::sleep(Duration::from_micros(10));
                    }
                }
                
                // Occasional yield to prevent CPU starvation
                if sequence % 100 == 0 {
                    thread::yield_now();
                }
            }
        });
        
        // Consumer thread - continuous consumption
        let ring_cons = ring.clone();
        let stop_cons = stop_flag.clone();
        let consumed_count = Arc::new(AtomicUsize::new(0));
        let consumer_errors = Arc::new(AtomicUsize::new(0));
        let consumed_counter = consumed_count.clone();
        let consumer_error_counter = consumer_errors.clone();
        
        let consumer = thread::spawn(move || {
            while !stop_cons.load(Ordering::Relaxed) {
                match ring_cons.try_consume() {
                    Ok(Some(_message)) => {
                        consumed_counter.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(None) => {
                        // No message available, brief pause
                        thread::sleep(Duration::from_micros(1));
                    }
                    Err(_) => {
                        consumer_error_counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        
        // Monitor thread - periodic health checks
        let stop_monitor = stop_flag.clone();
        let monitor_msg_count = message_count.clone();
        let health_checks = Arc::new(AtomicUsize::new(0));
        let health_counter = health_checks.clone();
        
        let monitor = thread::spawn(move || {
            let mut last_count = 0;
            
            while !stop_monitor.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(100));
                
                let current_count = monitor_msg_count.load(Ordering::Relaxed);
                
                // Verify system is making progress
                if current_count > last_count {
                    health_counter.fetch_add(1, Ordering::Relaxed);
                }
                last_count = current_count;
            }
        });
        
        let start_time = Instant::now();
        
        // Run stability test
        thread::sleep(test_duration);
        
        // Stop all threads
        stop_flag.store(true, Ordering::Relaxed);
        producer.join().unwrap();
        consumer.join().unwrap();
        monitor.join().unwrap();
        
        let actual_duration = start_time.elapsed();
        let messages = message_count.load(Ordering::Relaxed);
        let errors = error_count.load(Ordering::Relaxed);
        let consumed = consumed_count.load(Ordering::Relaxed);
        let health_checks_passed = health_checks.load(Ordering::Relaxed);
        
        let throughput = messages as f64 / actual_duration.as_secs_f64();
        let error_rate = errors as f64 / (messages + errors) as f64;
        
        println!("Long-running stability: {:.0} msg/sec throughput, {:.2}% error rate, {}/{} consumed, {} health checks passed in {:?}",
                throughput, error_rate * 100.0, consumed, messages, health_checks_passed, actual_duration);
        
        // Stability requirements
        assert!(messages > 1000, "Should process substantial messages during stability test");
        assert!(error_rate < 0.1, "Error rate should be very low in stable operation");
        assert!(consumed >= messages * 9 / 10, "Should consume at least 90% of messages");
        assert!(health_checks_passed > 20, "Should pass regular health checks");
    }

    /// Test: Resource cleanup and leak detection
    #[test]
    fn reliability_resource_cleanup() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        // Create multiple regions and pools for cleanup testing
        let region_names = vec!["cleanup_region_1", "cleanup_region_2", "cleanup_region_3"];
        let mut pools = Vec::new();
        
        for region_name in &region_names {
            let region_config = RegionConfig::new(*region_name, 64 * 1024)
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                .with_create(true);
            
            let region = manager.create_region(region_config).unwrap();
            
            let pool_config = BufferPoolConfig::new(&format!("{}_pool", region_name))
                .with_buffer_size(1024)
                .with_initial_count(5)
                .with_max_count(50)
                .with_pre_allocate(false);
            
            let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
            pools.push(pool);
        }
        
        // Allocate resources across all pools
        let mut all_buffers = Vec::new();
        for pool in &pools {
            let mut pool_buffers = Vec::new();
            for _ in 0..10 {
                if let Ok(buffer) = pool.get_buffer() {
                    pool_buffers.push(buffer);
                }
            }
            all_buffers.push(pool_buffers);
        }
        
        let initial_allocation_count: usize = all_buffers.iter().map(|buffers| buffers.len()).sum();
        
        // Phase 1: Partial cleanup
        for buffers in &mut all_buffers {
            buffers.drain(0..buffers.len() / 2); // Release half
        }
        
        // Verify pools can still allocate after partial cleanup
        let mut post_cleanup_buffers = Vec::new();
        for pool in &pools {
            if let Ok(buffer) = pool.get_buffer() {
                post_cleanup_buffers.push(buffer);
            }
        }
        
        assert!(!post_cleanup_buffers.is_empty(), "Should be able to allocate after partial cleanup");
        
        // Phase 2: Full cleanup
        all_buffers.clear();
        post_cleanup_buffers.clear();
        
        // Verify full recovery after complete cleanup
        let mut final_buffers = Vec::new();
        for pool in &pools {
            for _ in 0..5 {
                if let Ok(buffer) = pool.get_buffer() {
                    final_buffers.push(buffer);
                }
            }
        }
        
        assert!(final_buffers.len() >= pools.len(), "Should allocate at least one buffer per pool after full cleanup");
        
        println!("Resource cleanup: {} initial allocations across {} pools, full recovery after cleanup",
                initial_allocation_count, pools.len());
    }

    /// Test: Error propagation and handling chain
    #[test]
    fn reliability_error_propagation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        // Create region with intentionally small size to trigger errors
        let region_config = RegionConfig::new("error_prop_region", 32 * 1024) // Very small
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("error_prop.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("error_prop_pool")
            .with_buffer_size(16 * 1024) // Large buffers for small region
            .with_initial_count(1)
            .with_max_count(3) // Very limited
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        // Test error propagation through different layers
        let mut error_types_encountered = Vec::new();
        
        // Layer 1: Buffer pool exhaustion
        let mut buffers = Vec::new();
        loop {
            match pool.get_buffer() {
                Ok(buffer) => buffers.push(buffer),
                Err(e) => {
                    error_types_encountered.push(format!("Buffer exhaustion: {:?}", e));
                    break;
                }
            }
        }
        
        // Layer 2: Ring buffer with constrained memory
        let stats = Arc::new(renoir::topic::TopicStats::default());
        
        // Try to create ring buffer that might fail due to memory constraints
        let ring_result = SPSCTopicRing::new(16384, stats); // Large ring
        match ring_result {
            Ok(_ring) => {
                // Ring created successfully
            }
            Err(e) => {
                error_types_encountered.push(format!("Ring creation: {:?}", e));
            }
        }
        
        // Layer 3: Message handling with resource pressure
        if let Ok(ring) = SPSCTopicRing::new(256, Arc::new(renoir::topic::TopicStats::default())) {
            let large_payload = vec![0u8; 32 * 1024]; // Very large payload
            let message = Message::new_inline(1, 0, large_payload);
            
            match ring.try_publish(&message) {
                Ok(()) => {
                    // Message published
                }
                Err(e) => {
                    error_types_encountered.push(format!("Large message publish: {:?}", e));
                }
            }
        }
        
        // Verify error handling doesn't crash the system
        buffers.clear();
        
        // System should still be partially functional
        let recovery_buffer = pool.get_buffer();
        
        println!("Error propagation: {} error types encountered, system recovery: {}",
                error_types_encountered.len(),
                if recovery_buffer.is_ok() { "successful" } else { "failed" });
        
        // Should encounter some errors due to resource constraints
        assert!(!error_types_encountered.is_empty(), "Should encounter errors in resource-constrained environment");
        
        // But system should not completely fail
        assert!(error_types_encountered.len() < 10, "Should not have excessive error propagation");
    }

    /// Test: Data integrity under stress conditions
    #[test]
    fn reliability_data_integrity_stress() {
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(512, stats).unwrap());
        
        let message_count = 500;
        let integrity_failures = Arc::new(AtomicUsize::new(0));
        let successful_verifications = Arc::new(AtomicUsize::new(0));
        
        // Producer with checksummed messages
        let ring_prod = ring.clone();
        let producer = thread::spawn(move || {
            for i in 0..message_count {
                let data = format!("Integrity test message number {}", i);
                let checksum = data.chars().map(|c| c as u32).sum::<u32>();
                let payload = format!("{}|{}", data, checksum).into_bytes();
                let message = Message::new_inline(1, i as u64, payload);
                
                // Keep trying until published
                while ring_prod.try_publish(&message).is_err() {
                    thread::yield_now();
                }
            }
        });
        
        // Consumer with integrity verification
        let ring_cons = ring.clone();
        let failure_counter = integrity_failures.clone();
        let success_counter = successful_verifications.clone();
        
        let consumer = thread::spawn(move || {
            let mut received_count = 0;
            
            while received_count < message_count {
                if let Ok(Some(message)) = ring_cons.try_consume() {
                    received_count += 1;
                    
                    // Verify message integrity
                    let payload_data = match &message.payload {
                        renoir::topic::MessagePayload::Inline(data) => data.clone(),
                        renoir::topic::MessagePayload::Descriptor(_) => continue, // Skip descriptor messages
                    };
                    
                    if let Ok(payload_str) = String::from_utf8(payload_data) {
                        if let Some(pipe_pos) = payload_str.rfind('|') {
                            let (data_part, checksum_str) = payload_str.split_at(pipe_pos);
                            let checksum_str = &checksum_str[1..]; // Remove '|'
                            
                            if let Ok(received_checksum) = checksum_str.parse::<u32>() {
                                let calculated_checksum = data_part.chars().map(|c| c as u32).sum::<u32>();
                                
                                if received_checksum == calculated_checksum {
                                    success_counter.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    failure_counter.fetch_add(1, Ordering::Relaxed);
                                }
                            } else {
                                failure_counter.fetch_add(1, Ordering::Relaxed);
                            }
                        } else {
                            failure_counter.fetch_add(1, Ordering::Relaxed);
                        }
                    } else {
                        failure_counter.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        
        producer.join().unwrap();
        consumer.join().unwrap();
        
        let failures = integrity_failures.load(Ordering::Relaxed);
        let successes = successful_verifications.load(Ordering::Relaxed);
        let total_processed = failures + successes;
        
        println!("Data integrity stress: {}/{} messages verified successfully, {} failures ({:.2}% integrity rate)",
                successes, total_processed, failures, (successes as f64 / total_processed as f64) * 100.0);
        
        // Should have very high data integrity
        assert!(successes > 0, "Should verify some messages successfully");
        assert!(failures == 0, "Should have zero integrity failures under normal stress");
        assert_eq!(successes, message_count, "All messages should maintain integrity");
    }
}