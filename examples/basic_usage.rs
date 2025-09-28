#!/usr/bin/env rust

//! Basic usage example of the Renoir shared memory manager

use renoir::{
    memory::{SharedMemoryManager, RegionConfig, BackingType},
    buffers::{BufferPool, BufferPoolConfig},
    allocators::{BumpAllocator, Allocator},
    Result,
};
use std::{path::PathBuf, time::Duration};

fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();
    
    println!("Renoir Shared Memory Manager Example");
    println!("====================================");

    // Create a shared memory manager
    let manager = SharedMemoryManager::new();
    
    // Create a shared memory region
    let region_config = RegionConfig {
        name: "example_region".to_string(),
        size: 1024 * 1024, // 1MB
        backing_type: BackingType::FileBacked,
        file_path: Some(PathBuf::from("/tmp/renoir_example")),
        create: true,
        permissions: 0o644,
    };

    println!("Creating shared memory region: {}", region_config.name);
    let region = manager.create_region(region_config)?;
    
    println!("Region created successfully!");
    println!("  Name: {}", region.name());
    println!("  Size: {} bytes", region.size());

    // Create a buffer pool
    let pool_config = BufferPoolConfig {
        name: "example_pool".to_string(),
        buffer_size: 4096, // 4KB buffers
        initial_count: 8,
        max_count: 32,
        alignment: 64,
        pre_allocate: true,
        allocation_timeout: Some(Duration::from_millis(100)),
    };

    println!("\nCreating buffer pool: {}", pool_config.name);
    let buffer_pool = BufferPool::new(pool_config, region.clone())?;

    println!("Buffer pool created successfully!");
    println!("  Available buffers: {}", buffer_pool.available_count());

    // Allocate and use buffers
    println!("\nAllocating buffers...");
    let mut buffers = Vec::new();
    
    for i in 0..5 {
        let mut buffer = buffer_pool.get_buffer()?;
        
        // Write some data to the buffer
        let data = format!("Hello from buffer {}", i);
        let bytes = data.as_bytes();
        
        if bytes.len() <= buffer.size() {
            buffer.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
            buffer.resize(bytes.len())?;
            buffer.set_sequence(i as u64);
        }
        
        println!("  Buffer {}: {} bytes, sequence {}", 
                 i, buffer.size(), buffer.sequence());
        
        buffers.push(buffer);
    }

    // Check pool statistics
    let stats = buffer_pool.stats();
    println!("\nBuffer Pool Statistics:");
    println!("  In use: {}", stats.currently_in_use);
    println!("  Available: {}", buffer_pool.available_count());
    println!("  Total allocations: {}", stats.total_allocations);
    println!("  Success rate: {:.2}%", stats.success_rate() * 100.0);

    // Return buffers to pool
    println!("\nReturning buffers to pool...");
    for (i, buffer) in buffers.into_iter().enumerate() {
        // Read data from buffer before returning
        let data = String::from_utf8_lossy(&buffer.as_slice());
        println!("  Buffer {}: '{}'", i, data);
        
        buffer_pool.return_buffer(buffer)?;
    }

    println!("  Available buffers after return: {}", buffer_pool.available_count());

    // Demonstrate bump allocator
    println!("\nTesting bump allocator...");
    let memory_slice = unsafe {
        std::slice::from_raw_parts_mut(
            region.as_mut_ptr_unsafe::<u8>().offset(1024) as *mut u8,
            4096
        )
    };
    
    let bump_allocator = BumpAllocator::new(memory_slice)?;
    
    println!("  Total size: {}", bump_allocator.total_size());
    println!("  Used size: {}", bump_allocator.used_size());
    
    // Allocate some memory chunks
    for i in 0..5 {
        let size = 64 * (i + 1);
        let ptr = bump_allocator.allocate(size, 8)?;
        println!("  Allocated {} bytes at {:p}", size, ptr.as_ptr());
    }
    
    println!("  Used size after allocations: {}", bump_allocator.used_size());
    println!("  Available size: {}", bump_allocator.available_size());

    println!("\nExample completed successfully!");
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_example_functionality() {
        // Run a simplified version of the example for testing
        let manager = SharedMemoryManager::new();
        
        let region_config = RegionConfig {
            name: "test_region".to_string(),
            size: 64 * 1024, // 64KB
            backing_type: BackingType::FileBacked,
            file_path: Some(PathBuf::from("/tmp/renoir_test")),
            create: true,
            permissions: 0o644,
        };

        let region = manager.create_region(region_config).unwrap();
        assert_eq!(region.size(), 64 * 1024);
        
        let pool_config = BufferPoolConfig {
            name: "test_pool".to_string(),
            buffer_size: 1024,
            initial_count: 4,
            max_count: 8,
            alignment: 8,
            pre_allocate: true,
            allocation_timeout: Some(Duration::from_millis(50)),
        };

        let buffer_pool = BufferPool::new(pool_config, region).unwrap();
        assert_eq!(buffer_pool.available_count(), 4);
        
        let buffer = buffer_pool.get_buffer().unwrap();
        assert_eq!(buffer.size(), 1024);
        
        buffer_pool.return_buffer(buffer).unwrap();
        assert_eq!(buffer_pool.available_count(), 4);
    }
}