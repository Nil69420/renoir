//! Shared buffer pool system for large message payloads
//!
//! This module provides descriptor-based storage where ring buffers contain
//! small descriptors pointing to actual large data stored in shared buffer pools.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock, Mutex, atomic::{AtomicU32, AtomicUsize, Ordering}},
    time::SystemTime,
};

use crate::{
    error::{RenoirError, Result},
    memory::SharedMemoryRegion,
    buffer::{Buffer, BufferPool, BufferPoolConfig},
    topic::MessageDescriptor,
};

/// Unique identifier for buffer pools
pub type PoolId = u32;

/// Unique identifier for buffers within a pool  
pub type BufferHandle = u32;

/// Shared buffer pool manager for large message payloads
#[derive(Debug)]
pub struct SharedBufferPoolManager {
    /// Map of pool ID to buffer pools
    pools: RwLock<HashMap<PoolId, Arc<SharedBufferPool>>>,
    /// Next available pool ID
    next_pool_id: AtomicU32,
    /// Global statistics
    stats: SharedPoolStats,
}

impl SharedBufferPoolManager {
    /// Create a new shared buffer pool manager
    pub fn new() -> Self {
        Self {
            pools: RwLock::new(HashMap::new()),
            next_pool_id: AtomicU32::new(1),
            stats: SharedPoolStats::default(),
        }
    }

    /// Create a new buffer pool for large messages
    pub fn create_pool(
        &self,
        name: String,
        buffer_size: usize,
        max_buffers: usize,
        memory_region: Arc<SharedMemoryRegion>,
    ) -> Result<PoolId> {
        let pool_id = self.next_pool_id.fetch_add(1, Ordering::SeqCst);
        
        let config = BufferPoolConfig {
            name: name.clone(),
            buffer_size,
            max_count: max_buffers,
            initial_count: max_buffers / 4, // Pre-allocate 25%
            alignment: 8, // 8-byte alignment
            pre_allocate: true,
            allocation_timeout: None,
        };

        let pool = SharedBufferPool::new(pool_id, config, memory_region)?;
        
        let mut pools = self.pools.write().unwrap();
        pools.insert(pool_id, Arc::new(pool));
        
        self.stats.pools_created.fetch_add(1, Ordering::Relaxed);
        
        Ok(pool_id)
    }

    /// Get a buffer from a specific pool
    pub fn get_buffer(&self, pool_id: PoolId) -> Result<MessageDescriptor> {
        let pools = self.pools.read().unwrap();
        let pool = pools.get(&pool_id)
            .ok_or_else(|| RenoirError::invalid_parameter("pool_id", "Pool not found"))?;
            
        pool.get_buffer()
    }

    /// Return a buffer to its pool
    pub fn return_buffer(&self, descriptor: &MessageDescriptor) -> Result<()> {
        let pools = self.pools.read().unwrap();
        let pool = pools.get(&descriptor.pool_id)
            .ok_or_else(|| RenoirError::invalid_parameter("pool_id", "Pool not found"))?;
            
        pool.return_buffer(descriptor)
    }

    /// Get buffer data for reading/writing
    pub fn get_buffer_data(&self, descriptor: &MessageDescriptor) -> Result<Arc<Buffer>> {
        let pools = self.pools.read().unwrap();
        let pool = pools.get(&descriptor.pool_id)
            .ok_or_else(|| RenoirError::invalid_parameter("pool_id", "Pool not found"))?;
            
        pool.get_buffer_data(descriptor.buffer_handle)
    }

    /// Remove a buffer pool
    pub fn remove_pool(&self, pool_id: PoolId) -> Result<()> {
        let mut pools = self.pools.write().unwrap();
        
        if let Some(pool) = pools.remove(&pool_id) {
            // Ensure no buffers are still in use
            if pool.active_buffers() > 0 {
                pools.insert(pool_id, pool); // Put it back
                return Err(RenoirError::invalid_parameter(
                    "pool_id", 
                    "Cannot remove pool with active buffers"
                ));
            }
            
            self.stats.pools_destroyed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(RenoirError::invalid_parameter("pool_id", "Pool not found"))
        }
    }

    /// Get pool statistics
    pub fn pool_stats(&self, pool_id: PoolId) -> Result<(usize, usize, usize, usize)> {
        let pools = self.pools.read().unwrap();
        let pool = pools.get(&pool_id)
            .ok_or_else(|| RenoirError::invalid_parameter("pool_id", "Pool not found"))?;
            
        Ok(pool.stats())
    }

    /// Get global statistics
    pub fn global_stats(&self) -> &SharedPoolStats {
        &self.stats
    }

    /// List all active pools
    pub fn list_pools(&self) -> Vec<(PoolId, String)> {
        let pools = self.pools.read().unwrap();
        pools.iter()
            .map(|(id, pool)| (*id, pool.name().to_string()))
            .collect()
    }
}

impl Default for SharedBufferPoolManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Individual shared buffer pool for a specific size class
#[derive(Debug)]
pub struct SharedBufferPool {
    /// Pool identifier
    pool_id: PoolId,
    /// Pool name
    name: String,
    /// Underlying buffer pool
    buffer_pool: Arc<BufferPool>,
    /// Map of handle to buffer
    buffers: RwLock<HashMap<BufferHandle, Arc<Buffer>>>,
    /// Next available handle
    next_handle: AtomicU32,
    /// Active buffer count
    active_count: AtomicUsize,
    /// Pool statistics
    stats: Mutex<SharedPoolBufferStats>,
}

impl SharedBufferPool {
    /// Create a new shared buffer pool
    pub fn new(
        pool_id: PoolId,
        config: BufferPoolConfig,
        memory_region: Arc<SharedMemoryRegion>,
    ) -> Result<Self> {
        let buffer_pool = Arc::new(BufferPool::new(config.clone(), memory_region)?);
        
        Ok(Self {
            pool_id,
            name: config.name,
            buffer_pool,
            buffers: RwLock::new(HashMap::new()),
            next_handle: AtomicU32::new(1),
            active_count: AtomicUsize::new(0),
            stats: Mutex::new(SharedPoolBufferStats::default()),
        })
    }

    /// Get a buffer from this pool
    pub fn get_buffer(&self) -> Result<MessageDescriptor> {
        let buffer = self.buffer_pool.get_buffer()?;
        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
        
        // Store buffer reference
        {
            let mut buffers = self.buffers.write().unwrap();
            buffers.insert(handle, Arc::new(buffer));
        }
        
        self.active_count.fetch_add(1, Ordering::Relaxed);
        
        // Update statistics
        {
            let stats = self.stats.lock().unwrap();
            stats.buffers_allocated.fetch_add(1, Ordering::Relaxed);
            stats.current_active.fetch_add(1, Ordering::Relaxed);
        }

        let descriptor = MessageDescriptor::new(
            self.pool_id,
            handle,
            self.buffer_pool.config().buffer_size as u32,
        );

        Ok(descriptor)
    }

    /// Return a buffer to this pool
    pub fn return_buffer(&self, descriptor: &MessageDescriptor) -> Result<()> {
        if descriptor.pool_id != self.pool_id {
            return Err(RenoirError::invalid_parameter(
                "pool_id", 
                "Buffer does not belong to this pool"
            ));
        }

        // Check reference count
        if !descriptor.release() {
            return Ok(()); // Still has references
        }

        let buffer = {
            let mut buffers = self.buffers.write().unwrap();
            buffers.remove(&descriptor.buffer_handle)
                .ok_or_else(|| RenoirError::invalid_parameter(
                    "buffer_handle", 
                    "Buffer handle not found"
                ))?
        };

        // Return to underlying pool
        self.buffer_pool.return_buffer(Arc::try_unwrap(buffer).unwrap())?;
        
        self.active_count.fetch_sub(1, Ordering::Relaxed);

        // Update statistics
        {
            let stats = self.stats.lock().unwrap();
            stats.buffers_returned.fetch_add(1, Ordering::Relaxed);
            stats.current_active.fetch_sub(1, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Get buffer data for reading/writing
    pub fn get_buffer_data(&self, handle: BufferHandle) -> Result<Arc<Buffer>> {
        let buffers = self.buffers.read().unwrap();
        buffers.get(&handle)
            .cloned()
            .ok_or_else(|| RenoirError::invalid_parameter(
                "buffer_handle", 
                "Buffer handle not found"
            ))
    }

    /// Get the number of active buffers
    pub fn active_buffers(&self) -> usize {
        self.active_count.load(Ordering::Relaxed)
    }

    /// Get pool name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get pool statistics (returns atomic values)
    pub fn stats(&self) -> (usize, usize, usize, usize) {
        let stats = self.stats.lock().unwrap();
        (
            stats.buffers_allocated.load(Ordering::Relaxed),
            stats.buffers_returned.load(Ordering::Relaxed),
            stats.current_active.load(Ordering::Relaxed),
            stats.peak_active.load(Ordering::Relaxed),
        )
    }
}

/// Statistics for shared buffer pools
#[derive(Debug, Default)]
pub struct SharedPoolStats {
    /// Total pools created
    pub pools_created: AtomicUsize,
    /// Total pools destroyed
    pub pools_destroyed: AtomicUsize,
    /// Total bytes allocated across all pools
    pub total_bytes_allocated: AtomicUsize,
    /// Peak memory usage
    pub peak_memory_usage: AtomicUsize,
}

/// Statistics for individual buffer pools
#[derive(Debug, Default)]
pub struct SharedPoolBufferStats {
    /// Buffers allocated from pool
    pub buffers_allocated: AtomicUsize,
    /// Buffers returned to pool
    pub buffers_returned: AtomicUsize,
    /// Currently active buffers
    pub current_active: AtomicUsize,
    /// Peak active buffers
    pub peak_active: AtomicUsize,
    /// Average buffer lifetime (ms)
    pub avg_lifetime_ms: AtomicUsize,
    /// Pool creation time
    pub created_at: Option<SystemTime>,
}

/// Buffer pool registry for topic system
#[derive(Debug)]
pub struct BufferPoolRegistry {
    /// Pool manager
    manager: Arc<SharedBufferPoolManager>,
    /// Pool configurations by size class
    size_classes: RwLock<HashMap<usize, PoolId>>,
    /// Default memory region for pools
    default_region: Arc<SharedMemoryRegion>,
}

impl BufferPoolRegistry {
    /// Create a new buffer pool registry
    pub fn new(default_region: Arc<SharedMemoryRegion>) -> Self {
        Self {
            manager: Arc::new(SharedBufferPoolManager::new()),
            size_classes: RwLock::new(HashMap::new()),
            default_region,
        }
    }

    /// Get or create a pool for a specific size class
    pub fn get_pool_for_size(&self, size: usize) -> Result<PoolId> {
        // Round up to next power of 2 for size class
        let size_class = size.next_power_of_two();
        
        {
            let size_classes = self.size_classes.read().unwrap();
            if let Some(&pool_id) = size_classes.get(&size_class) {
                return Ok(pool_id);
            }
        }

        // Create new pool for this size class
        let pool_name = format!("pool_size_{}", size_class);
        let max_buffers = (1024 * 1024 * 64) / size_class; // 64MB per pool max
        let pool_id = self.manager.create_pool(
            pool_name,
            size_class,
            max_buffers.max(16), // At least 16 buffers
            self.default_region.clone(),
        )?;

        // Cache the mapping
        {
            let mut size_classes = self.size_classes.write().unwrap();
            size_classes.insert(size_class, pool_id);
        }

        Ok(pool_id)
    }

    /// Get buffer for payload size
    pub fn get_buffer_for_payload(&self, payload_size: usize) -> Result<MessageDescriptor> {
        let pool_id = self.get_pool_for_size(payload_size)?;
        self.manager.get_buffer(pool_id)
    }

    /// Return a buffer
    pub fn return_buffer(&self, descriptor: &MessageDescriptor) -> Result<()> {
        self.manager.return_buffer(descriptor)
    }

    /// Get buffer data
    pub fn get_buffer_data(&self, descriptor: &MessageDescriptor) -> Result<Arc<Buffer>> {
        self.manager.get_buffer_data(descriptor)
    }

    /// Get the pool manager
    pub fn manager(&self) -> &Arc<SharedBufferPoolManager> {
        &self.manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        memory::{SharedMemoryRegion, RegionConfig, BackingType},
        SharedMemoryManager,
    };

    fn create_test_region() -> Arc<SharedMemoryRegion> {
        let manager = SharedMemoryManager::new();
        let config = RegionConfig {
            name: "test_shared_pool".to_string(),
            size: 1024 * 1024, // 1MB
            backing_type: BackingType::MemFd,
            file_path: None,
            create: true,
            permissions: 0o600,
        };
        manager.create_region(config).expect("Failed to create region")
    }

    #[test]
    fn test_shared_pool_manager_creation() {
        let manager = SharedBufferPoolManager::new();
        let stats = manager.global_stats();
        assert_eq!(stats.pools_created.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_pool_creation_and_buffer_operations() {
        let manager = SharedBufferPoolManager::new();
        let region = create_test_region();

        // Create a pool
        let pool_id = manager.create_pool(
            "test_pool".to_string(),
            1024,
            10,
            region,
        ).unwrap();

        // Get a buffer
        let descriptor = manager.get_buffer(pool_id).unwrap();
        assert_eq!(descriptor.pool_id, pool_id);
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 1);

        // Get buffer data
        let buffer = manager.get_buffer_data(&descriptor).unwrap();
        assert_eq!(buffer.capacity(), 1024);

        // Return the buffer
        manager.return_buffer(&descriptor).unwrap();

        // Verify pool stats
        let (allocated, returned, active, _peak) = manager.pool_stats(pool_id).unwrap();
        assert_eq!(allocated, 1);
        assert_eq!(returned, 1);
        assert_eq!(active, 0);
    }

    #[test]
    fn test_buffer_pool_registry() {
        let region = create_test_region();
        let registry = BufferPoolRegistry::new(region);

        // Get pools for different size classes
        let pool_1k = registry.get_pool_for_size(1000).unwrap();
        let pool_2k = registry.get_pool_for_size(2000).unwrap();
        let pool_1k_again = registry.get_pool_for_size(1024).unwrap();

        // Same size class should return same pool
        assert_eq!(pool_1k, pool_1k_again);
        // Different size classes should return different pools
        assert_ne!(pool_1k, pool_2k);

        // Test buffer operations
        let descriptor = registry.get_buffer_for_payload(500).unwrap();
        let buffer_data = registry.get_buffer_data(&descriptor).unwrap();
        assert!(buffer_data.capacity() >= 500);

        registry.return_buffer(&descriptor).unwrap();
    }

    #[test]
    fn test_message_descriptor_ref_counting() {
        let descriptor = MessageDescriptor::new(1, 1, 1024);
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 1);

        // Add reference
        descriptor.add_ref();
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 2);

        // Release one reference
        assert!(!descriptor.release());
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 1);

        // Release final reference
        assert!(descriptor.release());
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 0);
    }
}