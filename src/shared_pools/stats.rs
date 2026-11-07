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

    /// Get current active buffer count
    pub fn active_count(&self) -> usize {
        self.current_active.load(Ordering::Relaxed)
    }
}
