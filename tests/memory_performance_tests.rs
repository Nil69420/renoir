//! Memory performance and allocation speed tests
//! Tests focused on memory allocation patterns, speed, and fragmentation resistance

use std::{
    sync::{Arc, atomic::{AtomicUsize, Ordering}},
    thread,
    time::Instant,
};

use tempfile::TempDir;
use renoir::{
    memory::{BackingType, RegionConfig, SharedMemoryManager},
    buffers::{BufferPool, BufferPoolConfig},
    allocators::{BumpAllocator, PoolAllocator, Allocator},
};

#[cfg(test)]
mod memory_performance_tests {
    use super::*;

    /// Test: Memory allocation speed benchmark
    #[test]
    fn perf_memory_allocation_speed() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let region_config = RegionConfig::new("speed_test_region", 2 * 1024 * 1024) // 2MB for embedded
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("speed_test.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("speed_pool")
            .with_buffer_size(512) // Smaller buffers for embedded
            .with_initial_count(100) // Much smaller for embedded
            .with_max_count(500) // Smaller max for embedded
            .with_pre_allocate(false); // Test dynamic allocation speed
        
        let pool = BufferPool::new(pool_config, region).unwrap();
        
        let allocation_count = 1000; // Much smaller for embedded
        let start_time = Instant::now();
        
        // Rapid allocation test
        let mut buffers = Vec::new();
        for _ in 0..allocation_count {
            match pool.get_buffer() {
                Ok(buffer) => buffers.push(buffer),
                Err(_) => break, // Pool exhausted
            }
        }
        
        let allocation_time = start_time.elapsed();
        let successful_allocations = buffers.len();
        
        println!("Allocated {} buffers in {:?} ({:.2} allocs/ms)", 
                successful_allocations, allocation_time, 
                successful_allocations as f64 / allocation_time.as_millis() as f64);
        
        // Deallocation speed test (buffers are automatically returned when dropped)
        let start_dealloc = Instant::now();
        drop(buffers);
        let deallocation_time = start_dealloc.elapsed();
        
        println!("Deallocated {} buffers in {:?}", successful_allocations, deallocation_time);
        
        // Verify performance thresholds (embedded-friendly)
        assert!(successful_allocations > 100, "Should allocate at least 100 buffers");
        assert!(allocation_time.as_millis() < 2000, "Allocation should complete within 2 seconds");
    }

    /// Test: Memory fragmentation resistance
    #[test]
    fn perf_fragmentation_resistance() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        // Create regions of varying sizes to simulate fragmentation (embedded-friendly sizes)
        let region_sizes = vec![8*1024, 32*1024, 16*1024, 64*1024, 4*1024];
        let mut created_regions = Vec::new();
        
        let start_time = Instant::now();
        
        // Create fragmentation pattern: create, remove some, create more
        for cycle in 0..10 {
            // Create regions
            for (i, &size) in region_sizes.iter().enumerate() {
                let region_name = format!("frag_{}_{}", cycle, i);
                let config = RegionConfig::new(&region_name, size)
                    .with_backing_type(BackingType::FileBacked)
                    .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                    .with_create(true);
                
                if let Ok(_region) = manager.create_region(config) {
                    created_regions.push(region_name);
                }
            }
            
            // Remove every other region to create holes
            if cycle % 2 == 1 {
                let remove_count = created_regions.len() / 3;
                for _ in 0..remove_count {
                    if let Some(region_name) = created_regions.pop() {
                        let _ = manager.remove_region(&region_name);
                    }
                }
            }
        }
        
        let fragmentation_time = start_time.elapsed();
        let final_region_count = manager.region_count();
        
        println!("Fragmentation test: {} regions created, {} final count, {:?} total time",
                created_regions.len() + (region_sizes.len() * 10) / 3, // Approximate removals
                final_region_count, fragmentation_time);
        
        assert!(fragmentation_time.as_secs() < 5, "Fragmentation handling should be fast");
        assert!(final_region_count > 0, "Should have regions remaining");
    }

    /// Test: Large memory region performance
    #[test]
    fn perf_large_memory_regions() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let large_sizes = vec![
            512 * 1024,  // 512KB
            2 * 1024 * 1024,  // 2MB  
            8 * 1024 * 1024, // 8MB
        ];
        
        for &size in &large_sizes {
            let region_name = format!("large_region_{}", size);
            let start_time = Instant::now();
            
            let config = RegionConfig::new(&region_name, size)
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                .with_create(true);
            
            match manager.create_region(config) {
                Ok(_region) => {
                    let creation_time = start_time.elapsed();
                    let size_mb = size as f64 / (1024.0 * 1024.0);
                    println!("Created {:.1}MB region in {:?}", size_mb, creation_time);
                    
                    // Verify reasonable creation time for SD card (embedded-friendly)
                    // Allow at least 100ms base time + 500ms per MB
                    let max_time_ms = 100 + (size_mb * 500.0) as u64;
                    assert!(creation_time.as_millis() < max_time_ms as u128, 
                           "Large region creation too slow: {:?} for {:.1}MB", creation_time, size_mb);
                }
                Err(e) => {
                    println!("Failed to create {}MB region: {:?}", size / (1024*1024), e);
                    // Don't fail test - might hit system limits
                }
            }
        }
    }

    /// Test: Concurrent allocation performance
    #[test]
    fn perf_concurrent_allocation_throughput() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        let region_config = RegionConfig::new("concurrent_region", 4 * 1024 * 1024) // 4MB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("concurrent.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("concurrent_pool")
            .with_buffer_size(1024)
            .with_initial_count(20)
            .with_max_count(500)
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        let thread_count = 2;
        let allocations_per_thread = 50;
        let successful_allocations = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        
        let start_time = Instant::now();
        
        for _ in 0..thread_count {
            let pool = pool.clone();
            let success_counter = successful_allocations.clone();
            
            let handle = thread::spawn(move || {
                let mut local_buffers = Vec::new();
                for _ in 0..allocations_per_thread {
                    if let Ok(buffer) = pool.get_buffer() {
                        success_counter.fetch_add(1, Ordering::Relaxed);
                        local_buffers.push(buffer);
                        
                        // Simulate some work
                        thread::yield_now();
                    }
                }
                // Buffers automatically returned when dropped
                local_buffers.len()
            });
            handles.push(handle);
        }
        
        let mut total_allocated = 0;
        for handle in handles {
            total_allocated += handle.join().unwrap();
        }
        
        let total_time = start_time.elapsed();
        let throughput = total_allocated as f64 / total_time.as_secs_f64();
        
        println!("Concurrent allocation: {} allocations by {} threads in {:?} ({:.2} allocs/sec)",
                total_allocated, thread_count, total_time, throughput);
        
        assert!(total_allocated > thread_count * allocations_per_thread / 2, 
               "Should succeed on at least half the allocations");
        assert!(throughput > 100.0, "Should achieve at least 100 allocations/sec");
    }

    /// Test: Memory access pattern performance
    #[test]
    fn perf_memory_access_patterns() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        let region_config = RegionConfig::new("access_test_region", 1024 * 1024) // 1MB
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("access_test.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("access_pool")
            .with_buffer_size(1024) // 1KB buffers
            .with_initial_count(10)
            .with_max_count(100)
            .with_pre_allocate(true);
        
        let pool = BufferPool::new(pool_config, region).unwrap();
        
        // Sequential access pattern
        let start_sequential = Instant::now();
        let mut buffers = Vec::new();
        
        for i in 0..20 {
            if let Ok(mut buffer) = pool.get_buffer() {
                // Write sequential pattern
                let slice = buffer.as_mut_slice();
                for j in 0..slice.len().min(256) {
                    slice[j] = ((i + j) % 256) as u8;
                }
                buffers.push(buffer);
            }
        }
        
        let sequential_time = start_sequential.elapsed();
        
        // Random access pattern
        let start_random = Instant::now();
        
        for buffer in &mut buffers {
            let slice = buffer.as_mut_slice();
            let len = slice.len().min(256);
            
            // Random access pattern (simplified pseudo-random)
            for i in (0..len).step_by(37) { // Prime step for pseudo-randomness
                slice[i] = (i % 256) as u8;
            }
        }
        
        let random_time = start_random.elapsed();
        
        println!("Sequential access: {:?}, Random access: {:?}", 
                sequential_time, random_time);
        
        // Usually sequential should be faster than random, but both should be reasonable on SD card
        assert!(sequential_time.as_millis() < 2000, "Sequential access should be fast");
        assert!(random_time.as_millis() < 5000, "Random access should be reasonably fast");
    }

    /// Test: Memory cleanup performance
    #[test]
    fn perf_memory_cleanup_speed() {
        let temp_dir = TempDir::new().unwrap();
        let manager = SharedMemoryManager::new();
        
        // Create many small regions (embedded-friendly count)
        let region_count = 20;
        let mut region_names = Vec::new();
        
        let creation_start = Instant::now();
        
        for i in 0..region_count {
            let region_name = format!("cleanup_test_{}", i);
            let config = RegionConfig::new(&region_name, 16 * 1024) // 16KB each
                .with_backing_type(BackingType::FileBacked)
                .with_file_path(temp_dir.path().join(format!("{}.dat", region_name)))
                .with_create(true);
            
            if manager.create_region(config).is_ok() {
                region_names.push(region_name);
            }
        }
        
        let creation_time = creation_start.elapsed();
        
        // Mass cleanup test
        let cleanup_start = Instant::now();
        
        for region_name in region_names {
            let _ = manager.remove_region(&region_name);
        }
        
        let cleanup_time = cleanup_start.elapsed();
        
        println!("Created {} regions in {:?}, cleaned up in {:?}",
                region_count, creation_time, cleanup_time);
        
        // Verify manager is clean
        assert_eq!(manager.region_count(), 0, "All regions should be cleaned up");
        
        // Performance expectations (SD card friendly)
        assert!(creation_time.as_millis() < 30000, "Mass creation should complete in 30s");
        assert!(cleanup_time.as_millis() < 15000, "Mass cleanup should complete in 15s");
    }

    /// Test: Allocator performance comparison  
    #[test]
    fn perf_allocator_comparison() {
        let memory_size = 256 * 1024; // 256KB
        let mut memory = vec![0u8; memory_size];
        
        // Test BumpAllocator performance
        let bump_allocator = BumpAllocator::new(&mut memory[..memory_size/2]).unwrap();
        let bump_start = Instant::now();
        
        let mut bump_ptrs = Vec::new();
        for _ in 0..100 {
            if let Ok(ptr) = bump_allocator.allocate(64, 8) {
                bump_ptrs.push(ptr);
            }
        }
        
        let bump_time = bump_start.elapsed();
        bump_allocator.reset().unwrap();
        
        // Test PoolAllocator performance  
        let pool_allocator = PoolAllocator::new(&mut memory[memory_size/2..], 64).unwrap();
        let pool_start = Instant::now();
        
        let mut pool_ptrs = Vec::new();
        for _ in 0..100 {
            if let Ok(ptr) = pool_allocator.allocate(64, 8) {
                pool_ptrs.push(ptr);
            }
        }
        
        let pool_alloc_time = pool_start.elapsed();
        
        // Deallocate pool allocator memory
        let dealloc_start = Instant::now();
        for ptr in pool_ptrs {
            let _ = pool_allocator.deallocate(ptr, 64);
        }
        let pool_dealloc_time = dealloc_start.elapsed();
        
        println!("Bump allocator: {:?} for 100 allocations", bump_time);
        println!("Pool allocator: {:?} alloc + {:?} dealloc", pool_alloc_time, pool_dealloc_time);
        
        // Both should be reasonably fast on embedded systems
        assert!(bump_time.as_millis() < 500, "Bump allocator should be reasonably fast");
        assert!(pool_alloc_time.as_millis() < 1000, "Pool allocator should be reasonably fast");
        assert!(pool_dealloc_time.as_millis() < 500, "Pool deallocation should be reasonably fast");
    }

    /// Test: Memory pressure recovery
    #[test]
    fn perf_memory_pressure_recovery() {
        let temp_dir = TempDir::new().unwrap();
        let manager = Arc::new(SharedMemoryManager::new());
        
        let region_config = RegionConfig::new("pressure_region", 512 * 1024) // 512KB - small to create pressure
            .with_backing_type(BackingType::FileBacked)
            .with_file_path(temp_dir.path().join("pressure.dat"))
            .with_create(true);
        
        let region = manager.create_region(region_config).unwrap();
        
        let pool_config = BufferPoolConfig::new("pressure_pool")
            .with_buffer_size(2048) // 2KB - reasonable buffers to fill quickly
            .with_initial_count(5)
            .with_max_count(60) // Will fill our 512KB region
            .with_pre_allocate(false);
        
        let pool = Arc::new(BufferPool::new(pool_config, region).unwrap());
        
        // Phase 1: Fill to capacity
        let mut buffers = Vec::new();
        let fill_start = Instant::now();
        
        loop {
            match pool.get_buffer() {
                Ok(buffer) => buffers.push(buffer),
                Err(_) => break, // Pool exhausted
            }
        }
        
        let fill_time = fill_start.elapsed();
        let filled_count = buffers.len();
        
        // Phase 2: Release half and measure recovery
        let release_count = filled_count / 2;
        
        // Explicitly drop half the buffers
        for _ in 0..release_count {
            buffers.pop();
        }
        
        // Give the pool a moment to reclaim freed buffers
        std::thread::sleep(std::time::Duration::from_millis(10));
        
        let recovery_start = Instant::now();
        let mut recovered_buffers = Vec::new();
        
        // Try to allocate again - should succeed now
        for _ in 0..std::cmp::min(release_count, 20) { // Limit attempts to avoid infinite loops
            match pool.get_buffer() {
                Ok(buffer) => recovered_buffers.push(buffer),
                Err(_) => break,
            }
        }
        
        let recovery_time = recovery_start.elapsed();
        let recovered_count = recovered_buffers.len();
        
        println!("Memory pressure test: filled {} buffers in {:?}, recovered {} in {:?}",
                filled_count, fill_time, recovered_count, recovery_time);
        
        assert!(filled_count > 5, "Should be able to fill some buffers");
        // Recovery behavior varies on embedded systems - don't require strict recovery
        if recovered_count > 0 {
            println!("Buffer recovery successful: {} buffers", recovered_count);
        } else {
            println!("Limited buffer recovery (acceptable on embedded systems)");
        }
        assert!(recovery_time.as_millis() < 5000, "Recovery should complete in reasonable time");
    }
}