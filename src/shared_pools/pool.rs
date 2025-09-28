//! Individual shared buffer pool implementation

use std::{
    collections::HashMap,
    sync::{Arc, RwLock, Mutex, atomic::{AtomicU32, AtomicUsize, Ordering}},
};

use crate::{
    error::{RenoirError, Result},
    memory::SharedMemoryRegion,
    buffers::{Buffer, BufferPool, BufferPoolConfig},
    topic::MessageDescriptor,
};

use super::manager::PoolId;
use super::stats::SharedPoolBufferStats;

/// Unique identifier for buffers within a pool  
pub type BufferHandle = u32;

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