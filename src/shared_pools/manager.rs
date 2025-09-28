//! Shared buffer pool manager for large message payloads

use std::{
    collections::HashMap,
    sync::{Arc, RwLock, atomic::{AtomicU32, Ordering}},
};

use crate::{
    error::{RenoirError, Result},
    memory::SharedMemoryRegion,
    buffers::{Buffer, BufferPoolConfig},
    topic::MessageDescriptor,
};

use super::stats::SharedPoolStats;
use super::pool::SharedBufferPool;

/// Unique identifier for buffer pools
pub type PoolId = u32;

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