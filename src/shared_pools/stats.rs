//! Statistics for shared buffer pools

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::SystemTime,
};

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

impl SharedPoolStats {
    /// Create new statistics instance
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Get current pool count (created - destroyed)
    pub fn active_pools(&self) -> usize {
        let created = self.pools_created.load(Ordering::Relaxed);
        let destroyed = self.pools_destroyed.load(Ordering::Relaxed);
        created.saturating_sub(destroyed)
    }
    
    /// Update memory statistics
    pub fn update_memory_usage(&self, bytes_allocated: usize) {
        self.total_bytes_allocated.fetch_add(bytes_allocated, Ordering::Relaxed);
        
        // Update peak if necessary
        let current = self.total_bytes_allocated.load(Ordering::Relaxed);
        let mut peak = self.peak_memory_usage.load(Ordering::Relaxed);
        while current > peak {
            match self.peak_memory_usage.compare_exchange_weak(
                peak,
                current,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => peak = x,
            }
        }
    }
    
    /// Get total memory allocated
    pub fn total_memory_allocated(&self) -> usize {
        self.total_bytes_allocated.load(Ordering::Relaxed)
    }
    
    /// Get peak memory usage
    pub fn peak_memory(&self) -> usize {
        self.peak_memory_usage.load(Ordering::Relaxed)
    }
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

impl SharedPoolBufferStats {
    /// Create new buffer statistics
    pub fn new() -> Self {
        Self {
            created_at: Some(SystemTime::now()),
            ..Default::default()
        }
    }
    
    /// Get allocation efficiency (returned / allocated)
    pub fn allocation_efficiency(&self) -> f64 {
        let allocated = self.buffers_allocated.load(Ordering::Relaxed);
        if allocated == 0 {
            return 1.0;
        }
        
        let returned = self.buffers_returned.load(Ordering::Relaxed);
        returned as f64 / allocated as f64
    }
    
    /// Get current active buffer count
    pub fn active_count(&self) -> usize {
        self.current_active.load(Ordering::Relaxed)
    }
    
    /// Update peak active count if necessary
    pub fn update_peak_active(&self) {
        let current = self.current_active.load(Ordering::Relaxed);
        let mut peak = self.peak_active.load(Ordering::Relaxed);
        
        while current > peak {
            match self.peak_active.compare_exchange_weak(
                peak,
                current,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => peak = x,
            }
        }
    }
    
    /// Get pool age in seconds
    pub fn age_seconds(&self) -> Option<u64> {
        self.created_at?.elapsed().ok().map(|d| d.as_secs())
    }
}