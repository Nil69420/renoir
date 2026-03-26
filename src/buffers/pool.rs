//! Buffer pool implementation for efficient memory management

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Instant, SystemTime},
};

use parking_lot::{Condvar, Mutex, RwLock};

use crate::{
    allocators::{Allocator, PoolAllocator},
    error::{RenoirError, Result},
    memory::SharedMemoryRegion,
};

use super::{
    buffer::Buffer,
    config::BufferPoolConfig,
    stats::{next_buffer_sequence, BufferPoolStats},
};

/// A pool of pre-allocated buffers for efficient memory management
#[derive(Debug)]
pub struct BufferPool {
    config: BufferPoolConfig,
    allocator: Arc<PoolAllocator>,
    available: Mutex<VecDeque<Buffer>>,
    buffer_returned: Condvar,
    allocated: RwLock<HashMap<usize, SystemTime>>,
    stats: RwLock<BufferPoolStats>,
}

impl BufferPool {
    /// Create a new buffer pool
    pub fn new(config: BufferPoolConfig, memory_region: Arc<SharedMemoryRegion>) -> Result<Self> {
        // Validate configuration
        config.validate()?;

        // Create pool allocator for the memory region
        let total_size = config.total_memory_required();
        if total_size > memory_region.size() {
            return Err(RenoirError::insufficient_space(
                total_size,
                memory_region.size(),
            ));
        }

        // SAFETY: `memory_region` provides a valid pointer for `total_size` bytes
        // and outlives the allocator via `Arc`.
        let allocator = unsafe {
            Arc::new(PoolAllocator::from_raw(
                memory_region.as_mut_ptr_unsafe::<u8>(),
                total_size,
                config.buffer_size,
            )?)
        };

        let mut available = VecDeque::new();

        // Pre-allocate buffers if requested
        if config.pre_allocate {
            for _ in 0..config.initial_count {
                let buffer = Buffer::new(
                    Arc::clone(&allocator) as Arc<dyn Allocator>,
                    config.buffer_size,
                    config.alignment,
                )?;
                available.push_back(buffer);
            }
        }

        let stats = BufferPoolStats {
            total_allocated: if config.pre_allocate {
                config.initial_count
            } else {
                0
            },
            currently_in_use: 0,
            peak_usage: 0,
            total_allocations: 0,
            total_deallocations: 0,
            allocation_failures: 0,
        };

        Ok(Self {
            config,
            allocator,
            available: Mutex::new(available),
            buffer_returned: Condvar::new(),
            allocated: RwLock::new(HashMap::new()),
            stats: RwLock::new(stats),
        })
    }

    /// Get a buffer from the pool
    pub fn get_buffer(&self) -> Result<Buffer> {
        // Try to get from available buffers first
        {
            let mut available = self.available.lock();
            if let Some(mut buffer) = available.pop_front() {
                buffer.set_sequence(next_buffer_sequence());
                self.track_allocation(&buffer);
                self.update_allocation_stats();
                return Ok(buffer);
            }
        }

        // Try to allocate a new buffer if under max limit
        if self.can_allocate_new() {
            if let Ok(buffer) = self.create_new_buffer() {
                self.track_allocation(&buffer);
                self.update_allocation_stats();
                self.increment_total_allocated();
                return Ok(buffer);
            }
        }

        // No buffer available — wait on Condvar if timeout is configured
        if let Some(timeout) = self.config.allocation_timeout {
            let deadline = Instant::now() + timeout;
            let mut available = self.available.lock();
            loop {
                if let Some(mut buffer) = available.pop_front() {
                    buffer.set_sequence(next_buffer_sequence());
                    self.track_allocation(&buffer);
                    self.update_allocation_stats();
                    return Ok(buffer);
                }

                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    self.record_allocation_failure();
                    return Err(RenoirError::buffer_full("buffer_pool"));
                }

                let result = self.buffer_returned.wait_for(&mut available, remaining);
                if result.timed_out() {
                    if let Some(mut buffer) = available.pop_front() {
                        buffer.set_sequence(next_buffer_sequence());
                        self.track_allocation(&buffer);
                        self.update_allocation_stats();
                        return Ok(buffer);
                    }
                    self.record_allocation_failure();
                    return Err(RenoirError::buffer_full("buffer_pool"));
                }
            }
        } else {
            // No timeout configured: block indefinitely until a buffer is returned.
            let mut available = self.available.lock();
            loop {
                if let Some(mut buffer) = available.pop_front() {
                    buffer.set_sequence(next_buffer_sequence());
                    self.track_allocation(&buffer);
                    self.update_allocation_stats();
                    return Ok(buffer);
                }

                // Wait until a buffer is returned, then re-check the queue.
                self.buffer_returned.wait(&mut available);
            }
        }
    }

    /// Return a buffer to the pool
    pub fn return_buffer(&self, buffer: Buffer) -> Result<()> {
        {
            let mut allocated = self.allocated.write();
            allocated.remove(&(buffer.sequence() as usize));
        }

        {
            let mut stats = self.stats.write();
            stats.currently_in_use = stats.currently_in_use.saturating_sub(1);
            stats.total_deallocations += 1;
        }

        {
            let mut available = self.available.lock();
            available.push_back(buffer);
            self.buffer_returned.notify_one();
        }

        Ok(())
    }

    /// Get current statistics
    pub fn stats(&self) -> BufferPoolStats {
        self.stats.read().clone()
    }

    /// Get pool configuration
    pub fn config(&self) -> &BufferPoolConfig {
        &self.config
    }

    /// Get number of available buffers
    pub fn available_count(&self) -> usize {
        self.available.lock().len()
    }

    /// Get number of allocated buffers
    pub fn allocated_count(&self) -> usize {
        self.allocated.read().len()
    }

    /// Check if pool can expand
    pub fn can_expand(&self) -> bool {
        let stats = self.stats.read();
        stats.total_allocated < self.config.max_count
    }

    /// Shrink pool by removing excess available buffers
    pub fn shrink(&self, target_available: usize) -> usize {
        let mut available = self.available.lock();
        let current_count = available.len();

        if current_count <= target_available {
            return 0;
        }

        let to_remove = current_count - target_available;
        let mut removed = 0;

        for _ in 0..to_remove {
            if available.pop_back().is_some() {
                removed += 1;
            }
        }

        // Update stats
        {
            let mut stats = self.stats.write();
            stats.total_allocated = stats.total_allocated.saturating_sub(removed);
        }

        removed
    }

    // Private helper methods

    fn can_allocate_new(&self) -> bool {
        let stats = self.stats.read();
        stats.total_allocated < self.config.max_count
    }

    fn create_new_buffer(&self) -> Result<Buffer> {
        let mut buffer = Buffer::new(
            Arc::clone(&self.allocator) as Arc<dyn Allocator>,
            self.config.buffer_size,
            self.config.alignment,
        )?;
        buffer.set_sequence(next_buffer_sequence());
        Ok(buffer)
    }

    fn track_allocation(&self, buffer: &Buffer) {
        let mut allocated = self.allocated.write();
        allocated.insert(buffer.sequence() as usize, SystemTime::now());
    }

    fn update_allocation_stats(&self) {
        let mut stats = self.stats.write();
        stats.currently_in_use += 1;
        stats.total_allocations += 1;
        if stats.currently_in_use > stats.peak_usage {
            stats.peak_usage = stats.currently_in_use;
        }
    }

    fn increment_total_allocated(&self) {
        let mut stats = self.stats.write();
        stats.total_allocated += 1;
    }

    fn record_allocation_failure(&self) {
        let mut stats = self.stats.write();
        stats.allocation_failures += 1;
    }
}

unsafe impl Send for BufferPool {}
unsafe impl Sync for BufferPool {}
