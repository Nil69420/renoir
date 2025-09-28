//! Buffer pool registry for topic system

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::{
    error::Result,
    memory::SharedMemoryRegion,
    buffers::Buffer,
    topic::MessageDescriptor,
};

use super::{
    manager::{SharedBufferPoolManager, PoolId},
};

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

    /// Get all registered size classes
    pub fn size_classes(&self) -> Vec<(usize, PoolId)> {
        let size_classes = self.size_classes.read().unwrap();
        size_classes.iter().map(|(&size, &pool_id)| (size, pool_id)).collect()
    }

    /// Get statistics for all pools
    pub fn all_pool_stats(&self) -> Vec<(PoolId, String, usize, usize, usize, usize)> {
        let pools = self.manager.list_pools();
        pools.into_iter()
            .filter_map(|(pool_id, name)| {
                self.manager.pool_stats(pool_id)
                    .ok()
                    .map(|(allocated, returned, active, peak)| {
                        (pool_id, name, allocated, returned, active, peak)
                    })
            })
            .collect()
    }

    /// Remove unused pools (pools with no active buffers)
    pub fn cleanup_unused_pools(&self) -> Result<usize> {
        let pools = self.manager.list_pools();
        let mut removed_count = 0;

        for (pool_id, _name) in pools {
            if let Ok((allocated, returned, _active, _peak)) = self.manager.pool_stats(pool_id) {
                // If all allocated buffers have been returned, pool can be removed
                if allocated > 0 && allocated == returned {
                    if self.manager.remove_pool(pool_id).is_ok() {
                        removed_count += 1;
                        
                        // Remove from size class mapping
                        let mut size_classes = self.size_classes.write().unwrap();
                        size_classes.retain(|_size, &mut pid| pid != pool_id);
                    }
                }
            }
        }

        Ok(removed_count)
    }
}