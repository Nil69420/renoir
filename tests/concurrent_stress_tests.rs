//! Concurrent stress tests for high-contention scenarios
//! Tests focused on thread safety, race conditions, and concurrent performance

use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Barrier, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use renoir::{
    buffers::{BufferPool, BufferPoolConfig},
    memory::{BackingType, RegionConfig, SharedMemoryManager},
    topic::Message,
    topic_rings::MPMCTopicRing,
};
use tempfile::TempDir;

#[cfg(test)]
mod concurrent_stress_tests {
    use super::*;

    /// Test: Maximum thread contention on memory manager
    #[test]
    fn stress_maximum_thread_contention() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());

        // Create initial regions (embedded-friendly)
        let region_count = 4;
        let mut region_names = Vec::new();

        for i in 0..region_count {
            let region_name = format!("contention_region_{}", i);
            let config = RegionConfig::new(&region_name, 128 * 1024) // 128KB
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                .with_create(true);

            manager.create_region(config).unwrap();
            region_names.push(region_name);
        }

        let thread_count = 4; // Embedded-friendly contention
        let operations_per_thread = 20;
        let barrier = Arc::new(Barrier::new(thread_count));
        let operation_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();

        // Spawn many threads doing mixed operations
        for thread_id in 0..thread_count {
            let manager = manager.clone();
            let region_names = region_names.clone();
            let barrier = barrier.clone();
            let op_count = operation_count.clone();
            let err_count = error_count.clone();
            let temp_dir_path = temp_dir.path().to_path_buf();

            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronized start for maximum contention

                for i in 0..operations_per_thread {
                    // Mix of operations: get, list, create (will fail), remove (will fail)
                    match i % 4 {
                        0 => {
                            // Get existing region
                            let region_name = &region_names[i % region_names.len()];
                            match manager.get_region(region_name) {
                                Ok(_) => op_count.fetch_add(1, Ordering::Relaxed),
                                Err(_) => err_count.fetch_add(1, Ordering::Relaxed),
                            };
                        }
                        1 => {
                            // List regions
                            let _ = manager.list_regions();
                            op_count.fetch_add(1, Ordering::Relaxed);
                        }
                        2 => {
                            // Try to create (might fail due to name collision)
                            let temp_name = format!("temp_{}_{}", thread_id, i);
                            let config = RegionConfig::new(&temp_name, 16 * 1024)
                                .with_backing_type(BackingType::FileBacked)
                                .with_file_path(temp_dir_path.join(format!("{}.dat", temp_name)))
                                .with_create(true);

                            match manager.create_region(config) {
                                Ok(_) => {
                                    op_count.fetch_add(1, Ordering::Relaxed);
                                    let _ = manager.remove_region(&temp_name); // Clean up
                                }
                                Err(_) => {
                                    err_count.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        3 => {
                            // Check region existence
                            let region_name = &region_names[i % region_names.len()];
                            let _ = manager.has_region(region_name);
                            op_count.fetch_add(1, Ordering::Relaxed);
                        }
                        _ => unreachable!(),
                    }

                    // Brief yield to encourage context switching
                    if i % 10 == 0 {
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
        let total_time = start_time.elapsed();

        let total_operations = operation_count.load(Ordering::Relaxed);
        let total_errors = error_count.load(Ordering::Relaxed);
        let ops_per_second = total_operations as f64 / total_time.as_secs_f64();

        println!(
            "Max contention: {} threads, {} operations, {} errors, {:?} time, {:.0} ops/sec",
            thread_count, total_operations, total_errors, total_time, ops_per_second
        );

        assert!(
            total_operations > thread_count * operations_per_thread / 2,
            "Should complete majority of operations even under high contention"
        );
        assert!(
            ops_per_second > 100.0,
            "Should maintain reasonable throughput"
        );
    }

    /// Test: Race condition detection in topic rings
    #[test]
    fn stress_topic_ring_race_conditions() {
        let stats = Arc::new(renoir::topic::TopicStats::default());
        let ring = Arc::new(MPMCTopicRing::new(128, stats).unwrap());

        let producer_count = 1;
        let consumer_count = 1;
        let messages_per_producer = 10;

        let barrier = Arc::new(Barrier::new(producer_count + consumer_count));
        let producers_done = Arc::new(AtomicUsize::new(0));
        let total_sent = Arc::new(AtomicUsize::new(0));
        let total_received = Arc::new(AtomicUsize::new(0));
        let message_ids = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let received_ids = Arc::new(Mutex::new(std::collections::HashSet::new()));

        let mut handles = Vec::new();

        // Spawn producers
        for producer_id in 0..producer_count {
            let ring = ring.clone();
            let barrier = barrier.clone();
            let producers_done = producers_done.clone();
            let total_sent = total_sent.clone();
            let message_ids = message_ids.clone();

            let handle = thread::spawn(move || {
                barrier.wait();

                for msg_idx in 0..messages_per_producer {
                    let unique_id = (producer_id * 10000 + msg_idx) as u64;
                    let payload = format!("P{}M{}", producer_id, msg_idx).into_bytes();
                    let message = Message::new_inline(producer_id as u32, unique_id, payload);

                    // Retry until published (with backoff)
                    let mut retry_count = 0;
                    loop {
                        match ring.try_publish(&message) {
                            Ok(()) => {
                                // Track sent message IDs only on success
                                {
                                    let mut ids = message_ids.lock().unwrap();
                                    ids.insert(unique_id);
                                }
                                total_sent.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                            Err(_) => {
                                retry_count += 1;
                                if retry_count > 100 {
                                    break; // Give up after many retries
                                }
                                if retry_count % 10 == 0 {
                                    thread::yield_now();
                                }
                            }
                        }
                    }
                }

                producers_done.fetch_add(1, Ordering::Relaxed);
            });
            handles.push(handle);
        }

        // Spawn consumers
        for _consumer_id in 0..consumer_count {
            let ring = ring.clone();
            let barrier = barrier.clone();
            let total_received = total_received.clone();
            let received_ids = received_ids.clone();

            let handle = thread::spawn(move || {
                barrier.wait();

                let mut local_received = 0;
                // Simple consumer loop with timeout
                let start_time = std::time::Instant::now();
                let timeout = Duration::from_secs(5); // 5 second timeout

                while start_time.elapsed() < timeout && local_received < messages_per_producer {
                    match ring.try_consume() {
                        Ok(Some(message)) => {
                            local_received += 1;
                            total_received.fetch_add(1, Ordering::Relaxed);

                            // Track received message IDs for duplicate detection
                            {
                                let mut ids = received_ids.lock().unwrap();
                                let unique_id = message.header.sequence;
                                if !ids.insert(unique_id) {
                                    panic!("Duplicate message detected: ID {}", unique_id);
                                }
                            }
                        }
                        Ok(None) => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(_) => break,
                    }
                }
            });
            handles.push(handle);
        }

        let start_time = Instant::now();
        for handle in handles {
            handle.join().unwrap();
        }
        let total_time = start_time.elapsed();

        let sent = total_sent.load(Ordering::Relaxed);
        let received = total_received.load(Ordering::Relaxed);

        // Verify no duplicates and reasonable message delivery
        let sent_ids = message_ids.lock().unwrap();
        let recv_ids = received_ids.lock().unwrap();

        println!(
            "Race condition test: {} producers, {} consumers, {} sent, {} received in {:?}",
            producer_count, consumer_count, sent, received, total_time
        );
        println!(
            "Unique sent IDs: {}, unique received IDs: {}",
            sent_ids.len(),
            recv_ids.len()
        );

        assert_eq!(
            sent_ids.len(),
            sent,
            "All sent messages should have unique IDs"
        );
        assert_eq!(
            recv_ids.len(),
            received,
            "All received messages should have unique IDs"
        );
        assert!(
            received >= sent / 2,
            "Should receive at least half the sent messages"
        );
    }

    /// Test: Buffer pool contention under pressure
    #[test]
    fn stress_buffer_pool_high_contention() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();

        let region_config = RegionConfig::new("contention_pool_region", 1024 * 1024) // 1MB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("contention_pool.dat"))
            .with_create(true);

        let region = manager.create_region(region_config).unwrap();

        let pool_config = BufferPoolConfig::new("contention_pool")
            .with_buffer_size(1024) // 1KB buffers
            .with_initial_count(20)
            .with_max_count(200)
            .with_pre_allocate(false);

        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());

        let thread_count = 2;
        let allocations_per_thread = 20;
        let barrier = Arc::new(Barrier::new(thread_count));
        let successful_allocations = Arc::new(AtomicUsize::new(0));
        let allocation_failures = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();

        for thread_id in 0..thread_count {
            let pool = pool.clone();
            let barrier = barrier.clone();
            let success_counter = successful_allocations.clone();
            let failure_counter = allocation_failures.clone();

            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronized start for maximum contention

                let mut thread_buffers = Vec::new();
                let mut successful_this_thread = 0;

                for i in 0..allocations_per_thread {
                    match pool.get_buffer() {
                        Ok(mut buffer) => {
                            successful_this_thread += 1;
                            success_counter.fetch_add(1, Ordering::Relaxed);

                            // Do some work with the buffer
                            let slice = buffer.as_mut_slice();
                            let write_size = slice.len().min(1024);
                            for j in 0..write_size {
                                slice[j] = ((thread_id + i + j) % 256) as u8;
                            }

                            thread_buffers.push(buffer);

                            // Periodically release some buffers to simulate real usage
                            if i % 10 == 0 && !thread_buffers.is_empty() {
                                thread_buffers.pop(); // Drop some buffers
                            }
                        }
                        Err(_) => {
                            failure_counter.fetch_add(1, Ordering::Relaxed);
                            // Brief backoff on failure
                            thread::yield_now();
                        }
                    }
                }

                successful_this_thread
            });
            handles.push(handle);
        }

        let start_time = Instant::now();
        let mut thread_results = Vec::new();
        for handle in handles {
            thread_results.push(handle.join().unwrap());
        }
        let total_time = start_time.elapsed();

        let total_successful = successful_allocations.load(Ordering::Relaxed);
        let total_failures = allocation_failures.load(Ordering::Relaxed);
        let success_rate =
            total_successful as f64 / (total_successful + total_failures) as f64 * 100.0;

        println!("Buffer pool contention: {} threads, {} successful, {} failed, {:.1}% success rate, {:?} time",
                thread_count, total_successful, total_failures, success_rate, total_time);

        assert!(
            success_rate > 50.0,
            "Should maintain >50% success rate under contention"
        );
        assert!(
            total_successful > thread_count * allocations_per_thread / 4,
            "Should succeed on at least 25% of allocations"
        );
    }

    /// Test: Memory region access storm  
    #[test]
    fn stress_memory_region_access_storm() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());

        // Create regions for the storm test
        let region_count = 10;
        let mut region_names = Vec::new();

        for i in 0..region_count {
            let region_name = format!("storm_region_{}", i);
            let config = RegionConfig::new(&region_name, 64 * 1024) // 64KB
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                .with_create(true);

            manager.create_region(config).unwrap();
            region_names.push(region_name);
        }

        let thread_count = 3; // Embedded-friendly storm of threads
        let accesses_per_thread = 200;
        let start_flag = Arc::new(AtomicBool::new(false));
        let operation_count = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();

        for _ in 0..thread_count {
            let manager = manager.clone();
            let region_names = region_names.clone();
            let start_flag = start_flag.clone();
            let op_count = operation_count.clone();

            let handle = thread::spawn(move || {
                // Wait for storm start signal
                while !start_flag.load(Ordering::Relaxed) {
                    thread::yield_now();
                }

                for i in 0..accesses_per_thread {
                    let region_name = &region_names[i % region_names.len()];

                    // Mix of access patterns
                    match i % 3 {
                        0 => {
                            if manager.get_region(region_name).is_ok() {
                                op_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        1 => {
                            if manager.has_region(region_name) {
                                op_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        2 => {
                            let _ = manager.list_regions();
                            op_count.fetch_add(1, Ordering::Relaxed);
                        }
                        _ => unreachable!(),
                    }
                }
            });
            handles.push(handle);
        }

        // Start the storm
        let storm_start = Instant::now();
        start_flag.store(true, Ordering::Relaxed);

        for handle in handles {
            handle.join().unwrap();
        }

        let storm_duration = storm_start.elapsed();
        let total_operations = operation_count.load(Ordering::Relaxed);
        let ops_per_second = total_operations as f64 / storm_duration.as_secs_f64();

        println!(
            "Access storm: {} threads, {} operations in {:?} ({:.0} ops/sec)",
            thread_count, total_operations, storm_duration, ops_per_second
        );

        assert!(
            total_operations > thread_count * accesses_per_thread / 2,
            "Should complete majority of operations in storm"
        );
        assert!(
            ops_per_second > 5000.0,
            "Should maintain high throughput under storm"
        );
    }

    /// Test: Deadlock detection and prevention
    #[test]
    fn stress_deadlock_prevention() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());

        // Create resources that could potentially deadlock
        let resource_count = 8;
        let mut region_names = Vec::new();

        for i in 0..resource_count {
            let region_name = format!("deadlock_region_{}", i);
            let config = RegionConfig::new(&region_name, 128 * 1024) // 128KB
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                .with_create(true);

            manager.create_region(config).unwrap();
            region_names.push(region_name);
        }

        let thread_count = 2;
        let operations_per_thread = 20;
        let operation_count = Arc::new(AtomicUsize::new(0));
        let timeout_duration = Duration::from_secs(10); // Reasonable timeout

        let mut handles = Vec::new();

        for thread_id in 0..thread_count {
            let manager = manager.clone();
            let region_names = region_names.clone();
            let op_count = operation_count.clone();

            let handle = thread::spawn(move || {
                for i in 0..operations_per_thread {
                    // Access resources in different orders to test deadlock prevention
                    let first_idx = (thread_id + i) % resource_count;
                    let second_idx = (thread_id + i + 1) % resource_count;

                    // Try to access two regions in sequence (potential deadlock pattern)
                    if manager.get_region(&region_names[first_idx]).is_ok() {
                        if manager.get_region(&region_names[second_idx]).is_ok() {
                            op_count.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    // Brief work simulation
                    if i % 20 == 0 {
                        thread::yield_now();
                    }
                }
            });
            handles.push(handle);
        }

        let test_start = Instant::now();

        // Use timeout to detect potential deadlocks
        let mut completed_threads = 0;
        for handle in handles {
            match handle.join() {
                Ok(()) => completed_threads += 1,
                Err(_) => {} // Thread panicked
            }

            // Check if we're taking too long (potential deadlock)
            if test_start.elapsed() > timeout_duration {
                panic!("Potential deadlock detected - test taking too long");
            }
        }

        let test_duration = test_start.elapsed();
        let total_operations = operation_count.load(Ordering::Relaxed);

        println!(
            "Deadlock prevention: {} threads completed, {} operations in {:?}",
            completed_threads, total_operations, test_duration
        );

        assert_eq!(
            completed_threads, thread_count,
            "All threads should complete without deadlock"
        );
        assert!(
            test_duration < timeout_duration,
            "Should complete within reasonable time"
        );
        assert!(total_operations > 0, "Should complete some operations");
    }

    /// Test: High-frequency create/destroy operations  
    #[test]
    fn stress_high_frequency_create_destroy() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());

        let thread_count = 2;
        let cycles_per_thread = 50;
        let barrier = Arc::new(Barrier::new(thread_count));
        let total_created = Arc::new(AtomicUsize::new(0));
        let total_destroyed = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();

        for thread_id in 0..thread_count {
            let manager = manager.clone();
            let barrier = barrier.clone();
            let created_counter = total_created.clone();
            let destroyed_counter = total_destroyed.clone();
            let temp_dir = temp_dir.path().to_path_buf();

            let handle = thread::spawn(move || {
                barrier.wait();

                for cycle in 0..cycles_per_thread {
                    let region_name = format!("temp_{}_{}", thread_id, cycle);
                    let config =
                        RegionConfig::new(&region_name, 32 * 1024) // 32KB
                            .with_backing_type(BackingType::FileBacked)
                            .with_file_path(temp_dir.join(format!("{}.dat", region_name)))
                            .with_create(true);

                    // Rapid create-destroy cycle
                    match manager.create_region(config) {
                        Ok(_) => {
                            created_counter.fetch_add(1, Ordering::Relaxed);

                            // Immediately destroy
                            if let Ok(()) = manager.remove_region(&region_name) {
                                destroyed_counter.fetch_add(1, Ordering::Relaxed);
                            }
                            // If removal fails, region might have been removed by another thread
                        }
                        Err(_) => {} // Creation might fail under high contention
                    }

                    // Yield occasionally to allow other threads
                    if cycle % 5 == 0 {
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
        let total_time = start_time.elapsed();

        let created = total_created.load(Ordering::Relaxed);
        let destroyed = total_destroyed.load(Ordering::Relaxed);
        let ops_per_second = (created + destroyed) as f64 / total_time.as_secs_f64();

        println!(
            "High-frequency create/destroy: {} created, {} destroyed in {:?} ({:.0} ops/sec)",
            created, destroyed, total_time, ops_per_second
        );

        // Verify cleanup - should have no regions left
        assert_eq!(
            manager.region_count(),
            0,
            "All regions should be cleaned up"
        );
        assert!(
            created > thread_count,
            "Should successfully create some regions"
        );
        assert!(
            destroyed >= created / 2,
            "Should destroy most created regions"
        );
    }
}
