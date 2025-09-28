//! Buffer management and pool implementations

use std::{
    sync::{Arc, Mutex, RwLock},
    collections::{HashMap, VecDeque},
    ptr::NonNull,
    time::{SystemTime, Duration},
};

use crate::{
    error::{RenoirError, Result},
    allocator::{Allocator, PoolAllocator},
    memory::SharedMemoryRegion,
};

/// A reference-counted buffer with metadata
#[derive(Debug, Clone)]
pub struct Buffer {
    /// Pointer to the buffer data
    data: NonNull<u8>,
    /// Size of the buffer in bytes
    size: usize,
    /// Capacity of the allocated buffer
    capacity: usize,
    /// Reference to the allocator that owns this buffer
    allocator: Arc<dyn Allocator>,
    /// Sequence number for ordering
    sequence: u64,
    /// Timestamp when buffer was allocated
    allocated_at: SystemTime,
    /// Optional user-defined metadata
    metadata: HashMap<String, String>,
}

impl Buffer {
    /// Create a new buffer from an allocator
    pub fn new(allocator: Arc<dyn Allocator>, size: usize, align: usize) -> Result<Self> {
        if size == 0 {
            return Err(RenoirError::invalid_parameter("size", "Buffer size cannot be zero"));
        }

        let capacity = size;
        let data = allocator.allocate(capacity, align)?;

        Ok(Self {
            data,
            size,
            capacity,
            allocator,
            sequence: 0,
            allocated_at: SystemTime::now(),
            metadata: HashMap::new(),
        })
    }

    /// Create a buffer from raw pointer and allocator
    pub unsafe fn from_raw(
        data: NonNull<u8>, 
        size: usize, 
        capacity: usize, 
        allocator: Arc<dyn Allocator>
    ) -> Self {
        Self {
            data,
            size,
            capacity,
            allocator,
            sequence: 0,
            allocated_at: SystemTime::now(),
            metadata: HashMap::new(),
        }
    }

    /// Get the buffer data as a slice
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.data.as_ptr(), self.size) }
    }

    /// Get the buffer data as a mutable slice
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.data.as_ptr(), self.size) }
    }

    /// Get the raw pointer to the buffer data
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// Get the raw mutable pointer to the buffer data
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.data.as_ptr()
    }

    /// Get the current size of valid data in the buffer
    pub fn size(&self) -> usize {
        self.size
    }

    /// Get the total capacity of the buffer
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Resize the buffer (only if new size <= capacity)
    pub fn resize(&mut self, new_size: usize) -> Result<()> {
        if new_size > self.capacity {
            return Err(RenoirError::invalid_parameter(
                "new_size", 
                "New size exceeds buffer capacity"
            ));
        }
        self.size = new_size;
        Ok(())
    }

    /// Get the sequence number
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Set the sequence number
    pub fn set_sequence(&mut self, sequence: u64) {
        self.sequence = sequence;
    }

    /// Get allocation timestamp
    pub fn allocated_at(&self) -> SystemTime {
        self.allocated_at
    }

    /// Add metadata
    pub fn set_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }

    /// Get metadata
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }

    /// Get all metadata
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }

    /// Clone the buffer (creates a new buffer with copied data)
    pub fn deep_clone(&self) -> Result<Self> {
        let mut new_buffer = Buffer::new(
            Arc::clone(&self.allocator), 
            self.capacity, 
            std::mem::align_of::<u8>()
        )?;
        
        new_buffer.as_mut_slice()[..self.size].copy_from_slice(self.as_slice());
        new_buffer.size = self.size;
        new_buffer.sequence = self.sequence;
        new_buffer.metadata = self.metadata.clone();
        
        Ok(new_buffer)
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        // Deallocate the buffer using the allocator
        let _ = self.allocator.deallocate(self.data, self.capacity, std::mem::align_of::<u8>());
    }
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

/// Configuration for buffer pools
#[derive(Debug, Clone)]
pub struct BufferPoolConfig {
    /// Name of the buffer pool
    pub name: String,
    /// Size of each buffer in bytes
    pub buffer_size: usize,
    /// Initial number of buffers to allocate
    pub initial_count: usize,
    /// Maximum number of buffers in the pool
    pub max_count: usize,
    /// Alignment requirement for buffers
    pub alignment: usize,
    /// Whether to pre-allocate all buffers
    pub pre_allocate: bool,
    /// Timeout for buffer allocation
    pub allocation_timeout: Option<Duration>,
}

impl Default for BufferPoolConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            buffer_size: 4096,
            initial_count: 16,
            max_count: 1024,
            alignment: std::mem::align_of::<u64>(),
            pre_allocate: true,
            allocation_timeout: Some(Duration::from_millis(100)),
        }
    }
}

/// A pool of pre-allocated buffers for efficient memory management
#[derive(Debug)]
pub struct BufferPool {
    /// Configuration
    config: BufferPoolConfig,
    /// Pool allocator
    allocator: Arc<PoolAllocator>,
    /// Available buffers
    available: Mutex<VecDeque<Buffer>>,
    /// Currently allocated buffers (for tracking)
    allocated: RwLock<HashMap<usize, SystemTime>>,
    /// Statistics
    stats: RwLock<BufferPoolStats>,
}

impl BufferPool {
    /// Create a new buffer pool
    pub fn new(config: BufferPoolConfig, memory_region: Arc<SharedMemoryRegion>) -> Result<Self> {
        if config.buffer_size == 0 {
            return Err(RenoirError::invalid_parameter("buffer_size", "Buffer size cannot be zero"));
        }

        if config.max_count == 0 {
            return Err(RenoirError::invalid_parameter("max_count", "Max count cannot be zero"));
        }

        // Create pool allocator for the memory region
        let total_size = config.buffer_size * config.max_count;
        if total_size > memory_region.size() {
            return Err(RenoirError::insufficient_space(total_size, memory_region.size()));
        }

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
            total_allocated: if config.pre_allocate { config.initial_count } else { 0 },
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
            allocated: RwLock::new(HashMap::new()),
            stats: RwLock::new(stats),
        })
    }

    /// Get a buffer from the pool
    pub fn get_buffer(&self) -> Result<Buffer> {
        let start_time = SystemTime::now();
        
        loop {
            // Try to get from available buffers first
            {
                let mut available = self.available.lock().unwrap();
                if let Some(mut buffer) = available.pop_front() {
                    buffer.set_sequence(self.next_sequence());
                    
                    // Track allocation
                    {
                        let mut allocated = self.allocated.write().unwrap();
                        allocated.insert(buffer.sequence() as usize, SystemTime::now());
                    }
                    
                    // Update stats
                    {
                        let mut stats = self.stats.write().unwrap();
                        stats.currently_in_use += 1;
                        stats.total_allocations += 1;
                        if stats.currently_in_use > stats.peak_usage {
                            stats.peak_usage = stats.currently_in_use;
                        }
                    }
                    
                    return Ok(buffer);
                }
            }

            // Try to allocate a new buffer if under max limit
            {
                let stats = self.stats.read().unwrap();
                if stats.total_allocated < self.config.max_count {
                    drop(stats);
                    
                    match Buffer::new(
                        Arc::clone(&self.allocator) as Arc<dyn Allocator>,
                        self.config.buffer_size,
                        self.config.alignment,
                    ) {
                        Ok(mut buffer) => {
                            buffer.set_sequence(self.next_sequence());
                            
                            // Track allocation
                            {
                                let mut allocated = self.allocated.write().unwrap();
                                allocated.insert(buffer.sequence() as usize, SystemTime::now());
                            }
                            
                            // Update stats
                            {
                                let mut stats = self.stats.write().unwrap();
                                stats.total_allocated += 1;
                                stats.currently_in_use += 1;
                                stats.total_allocations += 1;
                                if stats.currently_in_use > stats.peak_usage {
                                    stats.peak_usage = stats.currently_in_use;
                                }
                            }
                            
                            return Ok(buffer);
                        }
                        Err(_) => {
                            // Allocation failed, continue to timeout check
                        }
                    }
                }
            }

            // Check timeout
            if let Some(timeout) = self.config.allocation_timeout {
                if start_time.elapsed().unwrap_or(Duration::ZERO) > timeout {
                    let mut stats = self.stats.write().unwrap();
                    stats.allocation_failures += 1;
                    return Err(RenoirError::insufficient_space(
                        self.config.buffer_size, 
                        0
                    ));
                }
            } else {
                let mut stats = self.stats.write().unwrap();
                stats.allocation_failures += 1;
                return Err(RenoirError::insufficient_space(
                    self.config.buffer_size, 
                    0
                ));
            }

            // Brief wait before retrying
            std::thread::sleep(Duration::from_micros(10));
        }
    }

    /// Return a buffer to the pool
    pub fn return_buffer(&self, buffer: Buffer) -> Result<()> {
        // Verify this buffer belongs to our allocator
        if !self.allocator.owns(NonNull::new(buffer.as_mut_ptr()).unwrap()) {
            return Err(RenoirError::invalid_parameter(
                "buffer", 
                "Buffer does not belong to this pool"
            ));
        }

        // Remove from allocated tracking
        {
            let mut allocated = self.allocated.write().unwrap();
            allocated.remove(&(buffer.sequence() as usize));
        }

        // Return to available pool
        {
            let mut available = self.available.lock().unwrap();
            available.push_back(buffer);
        }

        // Update stats
        {
            let mut stats = self.stats.write().unwrap();
            stats.currently_in_use -= 1;
            stats.total_deallocations += 1;
        }

        Ok(())
    }

    /// Get pool statistics
    pub fn stats(&self) -> BufferPoolStats {
        self.stats.read().unwrap().clone()
    }

    /// Get pool configuration
    pub fn config(&self) -> &BufferPoolConfig {
        &self.config
    }

    /// Get the number of available buffers
    pub fn available_count(&self) -> usize {
        self.available.lock().unwrap().len()
    }

    /// Get the number of currently in-use buffers
    pub fn in_use_count(&self) -> usize {
        self.stats.read().unwrap().currently_in_use
    }

    /// Reset the pool (returns all buffers, clears stats)
    pub fn reset(&self) -> Result<()> {
        {
            let mut available = self.available.lock().unwrap();
            available.clear();
        }
        
        {
            let mut allocated = self.allocated.write().unwrap();
            allocated.clear();
        }

        self.allocator.reset()?;

        {
            let mut stats = self.stats.write().unwrap();
            *stats = BufferPoolStats::default();
        }

        Ok(())
    }

    /// Generate next sequence number
    fn next_sequence(&self) -> u64 {
        static SEQUENCE_COUNTER: std::sync::atomic::AtomicU64 = 
            std::sync::atomic::AtomicU64::new(1);
        SEQUENCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}

/// Statistics for buffer pool monitoring
#[derive(Debug, Clone, Default)]
pub struct BufferPoolStats {
    /// Total number of buffers ever allocated
    pub total_allocated: usize,
    /// Number of buffers currently in use
    pub currently_in_use: usize,
    /// Peak number of buffers in use simultaneously
    pub peak_usage: usize,
    /// Total number of buffer allocations requested
    pub total_allocations: u64,
    /// Total number of buffer deallocations
    pub total_deallocations: u64,
    /// Number of allocation failures
    pub allocation_failures: u64,
}

impl BufferPoolStats {
    /// Calculate allocation success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_allocations == 0 {
            return 1.0;
        }
        1.0 - (self.allocation_failures as f64 / self.total_allocations as f64)
    }

    /// Calculate pool utilization
    pub fn utilization(&self) -> f64 {
        if self.total_allocated == 0 {
            return 0.0;
        }
        self.currently_in_use as f64 / self.total_allocated as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{RegionConfig, BackingType};
    use tempfile::TempDir;

    #[test]
    fn test_buffer_creation() {
        let mut memory = vec![0u8; 1024];
        let allocator = Arc::new(PoolAllocator::new(&mut memory, 64).unwrap()) as Arc<dyn Allocator>;
        
        let buffer = Buffer::new(allocator, 32, 8).unwrap();
        assert_eq!(buffer.size(), 32);
        assert_eq!(buffer.capacity(), 32);
        assert!(buffer.sequence() == 0);
    }

    #[test]
    fn test_buffer_metadata() {
        let mut memory = vec![0u8; 1024];
        let allocator = Arc::new(PoolAllocator::new(&mut memory, 64).unwrap()) as Arc<dyn Allocator>;
        
        let mut buffer = Buffer::new(allocator, 32, 8).unwrap();
        buffer.set_metadata("topic".to_string(), "test_topic".to_string());
        
        assert_eq!(buffer.get_metadata("topic"), Some(&"test_topic".to_string()));
    }

    #[test]
    fn test_buffer_pool() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "test_pool".to_string(),
            size: 64 * 1024, // 64KB
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("pool_shm")),
            create: true,
            permissions: 0o644,
        };

        let region = Arc::new(crate::memory::SharedMemoryRegion::new(config).unwrap());
        
        let pool_config = BufferPoolConfig {
            name: "test".to_string(),
            buffer_size: 1024,
            initial_count: 4,
            max_count: 16,
            alignment: 8,
            pre_allocate: true,
            allocation_timeout: Some(Duration::from_millis(100)),
        };

        let pool = BufferPool::new(pool_config, region).unwrap();
        
        assert_eq!(pool.available_count(), 4);
        
        let buffer1 = pool.get_buffer().unwrap();
        assert_eq!(pool.available_count(), 3);
        assert_eq!(pool.in_use_count(), 1);
        
        pool.return_buffer(buffer1).unwrap();
        assert_eq!(pool.available_count(), 4);
        assert_eq!(pool.in_use_count(), 0);
    }

    #[test]
    fn test_buffer_pool_stats() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "test_stats".to_string(),
            size: 64 * 1024,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("stats_shm")),
            create: true,
            permissions: 0o644,
        };

        let region = Arc::new(crate::memory::SharedMemoryRegion::new(config).unwrap());
        
        let pool_config = BufferPoolConfig {
            buffer_size: 1024,
            initial_count: 2,
            max_count: 4,
            ..Default::default()
        };

        let pool = BufferPool::new(pool_config, region).unwrap();
        
        let _buffer1 = pool.get_buffer().unwrap();
        let _buffer2 = pool.get_buffer().unwrap();
        
        let stats = pool.stats();
        assert_eq!(stats.currently_in_use, 2);
        assert_eq!(stats.total_allocations, 2);
        assert!(stats.success_rate() > 0.99);
    }
}