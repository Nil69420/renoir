//! Security and edge case tests
//! Tests focused on security validation, boundary conditions, and unusual input handling

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
mod security_edge_tests {
    use super::*;

    /// Test: Handling of maximum size messages
    #[test]
    fn security_maximum_message_size() {
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(2048, stats).unwrap());
        
        // Test various large message sizes
        let test_sizes = vec![
            1024,     // Normal large message
            4096,     // Very large message
            16384,    // Extremely large message
            65536,    // Maximum practical size
        ];
        
        for &size in &test_sizes {
            let payload = vec![0xAAu8; size];
            let message = Message::new_inline(1, 0, payload);
            
            let result = ring.try_publish(&message);
            
            match result {
                Ok(()) => {
                    // Large message was accepted - verify we can consume it
                    match ring.try_consume() {
                        Ok(Some(consumed_msg)) => {
                            assert_eq!(consumed_msg.payload().len(), size, 
                                      "Consumed message should have original size");
                            assert!(consumed_msg.payload().iter().all(|&b| b == 0xAA),
                                   "Consumed message should have original content");
                        }
                        Ok(None) => {
                            panic!("Should be able to consume large message that was published");
                        }
                        Err(e) => {
                            println!("Failed to consume large message (size {}): {:?}", size, e);
                        }
                    }
                }
                Err(e) => {
                    // Large message was rejected - this is acceptable
                    println!("Large message rejected (size {}): {:?}", size, e);
                }
            }
        }
        
        // Verify system is still functional after large message attempts
        let small_payload = vec![0x42u8; 64];
        let small_message = Message::new_inline(1, 1, small_payload);
        let recovery_result = ring.try_publish(&small_message);
        
        assert!(recovery_result.is_ok(), "System should remain functional after large message tests");
    }

    /// Test: Handling of zero and empty inputs
    #[test]
    fn security_zero_empty_inputs() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        // Test empty/minimal region creation
        let minimal_config = RegionConfig::new("minimal_region", 4096) // 4KB minimum
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("minimal.dat"))
            .with_create(true);
        
        let minimal_region = manager.create_region(minimal_config).unwrap();
        
        // Test minimal buffer pool
        let minimal_pool_config = BufferPoolConfig::new("minimal_pool")
            .with_buffer_size(64) // Minimal buffer size
            .with_initial_count(1) // Minimal count
            .with_max_count(2) // Minimal growth
            .with_pre_allocate(false);
        
        let minimal_pool_result = BufferPool::new(minimal_pool_config, minimal_region);
        assert!(minimal_pool_result.is_ok(), "Should handle minimal buffer pool configuration");
        
        if let Ok(minimal_pool) = minimal_pool_result {
            // Test buffer allocation with minimal pool
            let buffer_result = minimal_pool.get_buffer();
            assert!(buffer_result.is_ok(), "Should allocate buffer from minimal pool");
        }
        
        // Test empty messages
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(256, stats).unwrap());
        
        let empty_payload = Vec::new();
        let empty_message = Message::new_inline(1, 0, empty_payload);
        let empty_result = ring.try_publish(&empty_message);
        
        match empty_result {
            Ok(()) => {
                // Empty message accepted - verify consumption
                if let Ok(Some(consumed)) = ring.try_consume() {
                    assert_eq!(consumed.payload().len(), 0, "Empty message should have zero payload");
                }
            }
            Err(_) => {
                // Empty message rejected - this is acceptable behavior
                println!("Empty message rejected (acceptable behavior)");
            }
        }
        
        // Test zero-sized ring (should fail gracefully)
        let zero_stats = Arc::new(renoir::topic::TopicStats::default());
        let zero_ring_result = SPSCTopicRing::new(0, zero_stats);
        
        match zero_ring_result {
            Ok(_) => {
                println!("Zero-size ring created (unexpected but not fatal)");
            }
            Err(_) => {
                println!("Zero-size ring rejected (expected behavior)");
            }
        }
        
        println!("Zero/empty input handling: System handles edge cases gracefully");
    }

    /// Test: Rapid allocation and deallocation patterns
    #[test]
    fn security_rapid_alloc_dealloc() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let region_config = RegionConfig::new("rapid_region", 512 * 1024) // 512KB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("rapid.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("rapid_pool")
            .with_buffer_size(1024)
            .with_initial_count(10)
            .with_max_count(400)
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        let cycles = 1000; // Rapid cycles
        let buffers_per_cycle = 20;
        
        let start_time = Instant::now();
        
        for cycle in 0..cycles {
            let mut cycle_buffers = Vec::new();
            
            // Rapid allocation
            for _ in 0..buffers_per_cycle {
                match pool.get_buffer() {
                    Ok(buffer) => cycle_buffers.push(buffer),
                    Err(_) => break, // Pool exhausted, continue
                }
            }
            
            // Immediate deallocation (drop all buffers)
            cycle_buffers.clear();
            
            // Occasional verification that pool is still functional
            if cycle % 100 == 0 {
                let test_buffer = pool.get_buffer();
                assert!(test_buffer.is_ok() || cycle > 500, 
                       "Pool should remain functional during rapid alloc/dealloc at cycle {}", cycle);
            }
        }
        
        let duration = start_time.elapsed();
        let operations_per_second = (cycles * buffers_per_cycle * 2) as f64 / duration.as_secs_f64(); // *2 for alloc+dealloc
        
        println!("Rapid alloc/dealloc: {} cycles, {:.0} operations/sec in {:?}",
                cycles, operations_per_second, duration);
        
        // Final verification
        let final_buffer = pool.get_buffer();
        assert!(final_buffer.is_ok(), "Pool should be functional after rapid allocation cycles");
    }

    /// Test: Concurrent access with adversarial patterns
    #[test]
    fn security_adversarial_concurrent_access() {
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(MPMCTopicRing::new(512, stats).unwrap());
        
        let test_duration = Duration::from_secs(3);
        let thread_count = 8;
        let stop_flag = Arc::new(AtomicBool::new(false));
        
        // Metrics
        let successful_operations = Arc::new(AtomicUsize::new(0));
        let failed_operations = Arc::new(AtomicUsize::new(0));
        let data_corruption_detected = Arc::new(AtomicUsize::new(0));
        
        let mut handles = Vec::new();
        
        for thread_id in 0..thread_count {
            let ring = ring.clone();
            let stop_flag = stop_flag.clone();
            let success_counter = successful_operations.clone();
            let failure_counter = failed_operations.clone();
            let corruption_counter = data_corruption_detected.clone();
            
            let handle = thread::spawn(move || {
                let mut operations = 0;
                
                while !stop_flag.load(Ordering::Relaxed) {
                    match thread_id % 4 {
                        0 => {
                            // Adversarial publisher - rapid publishing with verification data
                            let verification_data = format!("THREAD_{}_OP_{}", thread_id, operations);
                            let payload = format!("{}|CHECKSUM_{}", verification_data, 
                                                 verification_data.len()).into_bytes();
                            let message = Message::new_inline(thread_id as u32, operations as u64, payload);
                            
                            match ring.try_publish(&message) {
                                Ok(()) => success_counter.fetch_add(1, Ordering::Relaxed),
                                Err(_) => failure_counter.fetch_add(1, Ordering::Relaxed),
                            }
                        }
                        1 => {
                            // Adversarial consumer - rapid consumption with verification
                            match ring.try_consume() {
                                Ok(Some(message)) => {
                                    // Verify message integrity
                                    let payload_data = match &message.payload {
                                        renoir::topic::MessagePayload::Inline(data) => data.clone(),
                                        renoir::topic::MessagePayload::Descriptor(_) => continue,
                                    };
                                    
                                    if let Ok(payload_str) = String::from_utf8(payload_data) {
                                        if let Some(checksum_pos) = payload_str.find("|CHECKSUM_") {
                                            let (data_part, checksum_part) = payload_str.split_at(checksum_pos);
                                            let expected_len = data_part.len();
                                            
                                            if let Some(len_str) = checksum_part.strip_prefix("|CHECKSUM_") {
                                                if let Ok(actual_len) = len_str.parse::<usize>() {
                                                    if actual_len != expected_len {
                                                        corruption_counter.fetch_add(1, Ordering::Relaxed);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    success_counter.fetch_add(1, Ordering::Relaxed);
                                }
                                Ok(None) => {
                                    // No message available
                                }
                                Err(_) => {
                                    failure_counter.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        2 => {
                            // Adversarial mixed access - alternating rapid operations
                            if operations % 2 == 0 {
                                let payload = vec![thread_id as u8; 100];
                                let message = Message::new_inline(thread_id as u32, operations as u64, payload);
                                let _ = ring.try_publish(&message);
                            } else {
                                let _ = ring.try_consume();
                            }
                        }
                        3 => {
                            // Adversarial burst access - periods of intense activity
                            if operations % 50 < 10 {
                                // Burst period
                                for burst_op in 0..5 {
                                    let payload = format!("BURST_{}_{}", thread_id, burst_op).into_bytes();
                                    let message = Message::new_inline(thread_id as u32, burst_op as u64, payload);
                                    let _ = ring.try_publish(&message);
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                    
                    operations += 1;
                    
                    // Adversarial timing - occasional micro-sleeps and yields
                    match operations % 17 {
                        0 => thread::sleep(Duration::from_micros(1)),
                        5 => thread::yield_now(),
                        10 => {
                            // Brief busy wait
                            let busy_start = Instant::now();
                            while busy_start.elapsed() < Duration::from_micros(5) {
                                // Busy wait
                            }
                        }
                        _ => {}
                    }
                }
            });
            
            handles.push(handle);
        }
        
        let start_time = Instant::now();
        
        // Run adversarial test
        thread::sleep(test_duration);
        
        // Stop all threads
        stop_flag.store(true, Ordering::Relaxed);
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        let duration = start_time.elapsed();
        let successes = successful_operations.load(Ordering::Relaxed);
        let failures = failed_operations.load(Ordering::Relaxed);
        let corruptions = data_corruption_detected.load(Ordering::Relaxed);
        
        let total_attempts = successes + failures;
        let success_rate = if total_attempts > 0 { 
            successes as f64 / total_attempts as f64 
        } else { 
            0.0 
        };
        
        println!("Adversarial concurrent access: {}/{} operations successful ({:.1}% success rate), {} data corruptions detected in {:?}",
                successes, total_attempts, success_rate * 100.0, corruptions, duration);
        
        // Security requirements
        assert!(corruptions == 0, "Should not detect any data corruption under adversarial access");
        assert!(success_rate > 0.1, "Should achieve reasonable success rate even under adversarial conditions");
    }

    /// Test: Resource exhaustion and recovery patterns
    #[test]
    fn security_resource_exhaustion_recovery() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let region_config = RegionConfig::new("exhaustion_region", 128 * 1024) // Small region
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("exhaustion.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("exhaustion_pool")
            .with_buffer_size(4096)
            .with_initial_count(5)
            .with_max_count(30) // Limited capacity
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        // Phase 1: Controlled exhaustion
        let mut exhaustion_buffers = Vec::new();
        let mut exhaustion_count = 0;
        
        loop {
            match pool.get_buffer() {
                Ok(buffer) => {
                    exhaustion_buffers.push(buffer);
                    exhaustion_count += 1;
                }
                Err(_) => break,
            }
        }
        
        println!("Resource exhaustion: allocated {} buffers until exhaustion", exhaustion_count);
        
        // Phase 2: Verify system handles exhaustion gracefully
        for _ in 0..10 {
            let result = pool.get_buffer();
            assert!(result.is_err(), "Should consistently fail when exhausted");
        }
        
        // Phase 3: Gradual recovery testing
        let recovery_steps = vec![25, 50, 75, 100]; // Percentage recovery
        
        for &recovery_percent in &recovery_steps {
            let buffers_to_release = (exhaustion_count * recovery_percent) / 100;
            let release_count = std::cmp::min(buffers_to_release, exhaustion_buffers.len());
            
            // Release buffers
            for _ in 0..release_count {
                if !exhaustion_buffers.is_empty() {
                    exhaustion_buffers.pop();
                }
            }
            
            // Test recovery
            let mut recovery_buffers = Vec::new();
            for _ in 0..5 {
                if let Ok(buffer) = pool.get_buffer() {
                    recovery_buffers.push(buffer);
                }
            }
            
            let recovered_count = recovery_buffers.len();
            println!("Recovery at {}%: released {}, recovered {} new buffers", 
                    recovery_percent, release_count, recovered_count);
            
            assert!(recovered_count > 0 || recovery_percent < 50, 
                   "Should recover some buffers after releasing {}%", recovery_percent);
        }
        
        // Phase 4: Full recovery
        exhaustion_buffers.clear();
        
        let full_recovery_buffer = pool.get_buffer();
        assert!(full_recovery_buffer.is_ok(), "Should fully recover after releasing all buffers");
        
        println!("Resource exhaustion recovery: System successfully recovered");
    }

    /// Test: Boundary condition handling
    #[test]
    fn security_boundary_conditions() {
        // Test 1: Ring buffer at exact capacity
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let small_ring = Arc::new(SPSCTopicRing::new(8, stats).unwrap()); // Very small ring
        
        let small_payload = vec![0x42u8; 16];
        let mut publish_count = 0;
        
        // Fill to exact capacity
        while small_ring.try_publish(&Message::new_inline(1, publish_count, small_payload.clone())).is_ok() {
            publish_count += 1;
            if publish_count > 100 { // Safety limit
                break;
            }
        }
        
        println!("Boundary test: Small ring accepted {} messages before full", publish_count);
        
        // Verify one more fails
        let overflow_result = small_ring.try_publish(&Message::new_inline(1, 999, small_payload));
        assert!(overflow_result.is_err(), "Should reject message when at exact capacity");
        
        // Test 2: Buffer pool at exact limits
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let boundary_config = RegionConfig::new("boundary_region", 32768) // Exact power of 2
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("boundary.dat"))
            .with_create(true);
        
        let boundary_region = manager.create_region(boundary_config).unwrap();
        
        let boundary_pool_config = BufferPoolConfig::new("boundary_pool")
            .with_buffer_size(1024) // Exact division
            .with_initial_count(1)
            .with_max_count(16) // Exact fit for region
            .with_pre_allocate(false);
        
        let boundary_pool_result = BufferPool::new(boundary_pool_config, boundary_region);
        
        match boundary_pool_result {
            Ok(boundary_pool) => {
                // Test allocation at boundary
                let mut boundary_buffers = Vec::new();
                for _ in 0..20 { // Try to exceed limit
                    if let Ok(buffer) = boundary_pool.get_buffer() {
                        boundary_buffers.push(buffer);
                    }
                }
                
                println!("Boundary test: Pool allocated {} buffers at capacity limit", 
                        boundary_buffers.len());
                
                assert!(boundary_buffers.len() <= 16, "Should not exceed configured maximum");
            }
            Err(e) => {
                println!("Boundary test: Pool creation failed at exact limits: {:?}", e);
            }
        }
        
        // Test 3: Message with exact size boundaries
        let boundary_stats = Arc::new(renoir::topic::TopicStats::default());
        let boundary_ring = Arc::new(SPSCTopicRing::new(1024, boundary_stats).unwrap());
        
        let boundary_sizes = vec![0, 1, 255, 256, 1023, 1024, 4095, 4096];
        
        for &size in &boundary_sizes {
            let boundary_payload = vec![0x55u8; size];
            let boundary_message = Message::new_inline(1, size as u64, boundary_payload);
            let boundary_result = boundary_ring.try_publish(&boundary_message);
            
            match boundary_result {
                Ok(()) => {
                    println!("Boundary test: Message size {} accepted", size);
                    // Clear ring for next test
                    let _ = boundary_ring.try_consume();
                }
                Err(_) => {
                    println!("Boundary test: Message size {} rejected", size);
                }
            }
        }
        
        println!("Boundary condition handling: All tests completed");
    }

    /// Test: Input validation and sanitization
    #[test]
    fn security_input_validation() {
        // Test 1: Invalid region names and configurations
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let long_name = "a".repeat(1000);
        let invalid_names = vec![
            "", // Empty name
            &long_name, // Very long name
            "invalid/chars", // Invalid characters
            "null\0name", // Null character
        ];
        
        for invalid_name in invalid_names {
            let invalid_config = RegionConfig::new(invalid_name, 4096)
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join("invalid.dat"))
                .with_create(true);
            
            let result = manager.create_region(invalid_config);
            
            match result {
                Ok(_) => {
                    println!("Input validation: Invalid name '{}' was accepted (length: {})", 
                            if invalid_name.len() > 50 { "LONG_NAME" } else { &invalid_name }, 
                            invalid_name.len());
                }
                Err(_) => {
                    println!("Input validation: Invalid name rejected (good)");
                }
            }
        }
        
        // Test 2: Invalid buffer pool configurations
        let valid_region = manager.create_region(
            RegionConfig::new("valid_region", 64 * 1024)
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join("valid.dat"))
                .with_create(true)
        ).unwrap();
        
        let invalid_pool_configs = vec![
            ("empty_name", "", 1024, 1, 10),
            ("zero_buffer_size", "zero_pool", 0, 1, 10),
            ("huge_buffer", "huge_pool", 1024 * 1024, 1, 10), // 1MB buffer
            ("zero_initial", "zero_init", 1024, 0, 10),
            ("invalid_max", "invalid_max", 1024, 10, 5), // max < initial
        ];
        
        for (test_name, pool_name, buffer_size, initial_count, max_count) in invalid_pool_configs {
            let config = BufferPoolConfig::new(pool_name)
                .with_buffer_size(buffer_size)
                .with_initial_count(initial_count)
                .with_max_count(max_count)
                .with_pre_allocate(false);
            
            let result = BufferPool::new(config, valid_region.clone());
            
            match result {
                Ok(_) => {
                    println!("Input validation: {} configuration accepted", test_name);
                }
                Err(_) => {
                    println!("Input validation: {} configuration rejected (good)", test_name);
                }
            }
        }
        
        // Test 3: Invalid message data
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(SPSCTopicRing::new(512, stats).unwrap());
        
        let invalid_payloads = vec![
            vec![0u8; 0], // Empty
            vec![0xFFu8; 1024 * 1024], // Very large
            (0..256).map(|i| i as u8).collect(), // Pattern data
        ];
        
        for (i, payload) in invalid_payloads.iter().enumerate() {
            let message = Message::new_inline(i as u32, i as u64, payload.clone());
            let result = ring.try_publish(&message);
            
            match result {
                Ok(()) => {
                    println!("Input validation: Payload {} (size {}) accepted", i, payload.len());
                    // Clear for next test
                    let _ = ring.try_consume();
                }
                Err(_) => {
                    println!("Input validation: Payload {} (size {}) rejected", i, payload.len());
                }
            }
        }
        
        println!("Input validation: Security testing completed");
    }

    /// Test: Thread safety under extreme conditions
    #[test]
    fn security_extreme_thread_safety() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        let region_config = RegionConfig::new("extreme_region", 1024 * 1024) // 1MB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("extreme.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("extreme_pool")
            .with_buffer_size(1024)
            .with_initial_count(50)
            .with_max_count(200)
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(MPMCTopicRing::new(1024, stats).unwrap());
        
        let extreme_thread_count = 50; // Extreme thread count
        let test_duration = Duration::from_secs(2);
        let stop_flag = Arc::new(AtomicBool::new(false));
        
        // Shared metrics for thread safety validation
        let data_races_detected = Arc::new(AtomicUsize::new(0));
        let successful_operations = Arc::new(AtomicUsize::new(0));
        let thread_collisions = Arc::new(AtomicUsize::new(0));
        
        let mut handles = Vec::new();
        
        for thread_id in 0..extreme_thread_count {
            let pool = pool.clone();
            let ring = ring.clone();
            let stop_flag = stop_flag.clone();
            let race_detector = data_races_detected.clone();
            let success_counter = successful_operations.clone();
            let collision_counter = thread_collisions.clone();
            
            let handle = thread::spawn(move || {
                let mut thread_local_counter = 0u64;
                
                while !stop_flag.load(Ordering::Relaxed) {
                    match thread_id % 5 {
                        0 => {
                            // Extreme buffer operations
                            if let Ok(mut buffer) = pool.get_buffer() {
                                // Write thread signature and verify
                                let signature = thread_id as u8;
                                buffer.as_mut_slice().fill(signature);
                                
                                // Immediate verification
                                if !buffer.as_slice().iter().all(|&b| b == signature) {
                                    race_detector.fetch_add(1, Ordering::Relaxed);
                                }
                                
                                success_counter.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        1 => {
                            // Extreme ring operations
                            let payload = format!("EXTREME_{}_{}", thread_id, thread_local_counter).into_bytes();
                            let message = Message::new_inline(thread_id as u32, thread_local_counter, payload);
                            
                            if ring.try_publish(&message).is_ok() {
                                success_counter.fetch_add(1, Ordering::Relaxed);
                            }
                            
                            thread_local_counter += 1;
                        }
                        2 => {
                            // Extreme consumption with verification
                            if let Ok(Some(message)) = ring.try_consume() {
                                // Verify message format
                                let payload_data = match &message.payload {
                                    renoir::topic::MessagePayload::Inline(data) => data.clone(),
                                    renoir::topic::MessagePayload::Descriptor(_) => continue,
                                };
                                
                                if let Ok(payload_str) = String::from_utf8(payload_data) {
                                    if !payload_str.starts_with("EXTREME_") {
                                        race_detector.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                success_counter.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        3 => {
                            // Extreme mixed operations
                            let start_ops = success_counter.load(Ordering::Relaxed);
                            
                            // Rapid sequence of operations
                            for _ in 0..3 {
                                let _ = pool.get_buffer();
                                let payload = vec![thread_id as u8; 16];
                                let message = Message::new_inline(thread_id as u32, 0, payload);
                                let _ = ring.try_publish(&message);
                                let _ = ring.try_consume();
                            }
                            
                            let end_ops = success_counter.load(Ordering::Relaxed);
                            if end_ops > start_ops {
                                // Potential collision if multiple threads increment simultaneously
                                if end_ops - start_ops > 10 {
                                    collision_counter.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        4 => {
                            // Extreme timing-sensitive operations
                            let timing_start = Instant::now();
                            
                            if let Ok(_buffer) = pool.get_buffer() {
                                let allocation_time = timing_start.elapsed();
                                
                                // If allocation took too long, might indicate contention issues
                                if allocation_time > Duration::from_millis(1) {
                                    collision_counter.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    success_counter.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            });
            
            handles.push(handle);
        }
        
        let start_time = Instant::now();
        
        // Run extreme thread safety test
        thread::sleep(test_duration);
        
        // Stop all threads
        stop_flag.store(true, Ordering::Relaxed);
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        let duration = start_time.elapsed();
        let races = data_races_detected.load(Ordering::Relaxed);
        let successes = successful_operations.load(Ordering::Relaxed);
        let collisions = thread_collisions.load(Ordering::Relaxed);
        
        println!("Extreme thread safety: {} threads, {} successful ops, {} data races, {} collisions detected in {:?}",
                extreme_thread_count, successes, races, collisions, duration);
        
        // Thread safety requirements
        assert!(races == 0, "Should not detect any data races under extreme threading");
        assert!(successes > 0, "Should complete some operations under extreme threading");
    }
}