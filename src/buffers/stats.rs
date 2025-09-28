//! Buffer pool statistics tracking

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Statistics for buffer pool monitoring and performance analysis
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
    /// Create new statistics instance
    pub fn new() -> Self {
        Default::default()
    }

    /// Calculate allocation success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.total_allocations == 0 {
            return 1.0;
        }
        1.0 - (self.allocation_failures as f64 / self.total_allocations as f64)
    }

    /// Calculate pool utilization (0.0 to 1.0)
    pub fn utilization(&self) -> f64 {
        if self.total_allocated == 0 {
            return 0.0;
        }
        self.currently_in_use as f64 / self.total_allocated as f64
    }

    /// Get allocation failure rate (0.0 to 1.0)
    pub fn failure_rate(&self) -> f64 {
        if self.total_allocations == 0 {
            return 0.0;
        }
        self.allocation_failures as f64 / self.total_allocations as f64
    }

    /// Check if the pool is healthy (low failure rate, reasonable utilization)
    pub fn is_healthy(&self) -> bool {
        self.success_rate() > 0.95 && self.utilization() < 0.9
    }

    /// Get a summary string of the statistics
    pub fn summary(&self) -> String {
        format!(
            "BufferPoolStats {{ allocated: {}, in_use: {}, peak: {}, \
             allocations: {}, failures: {}, success_rate: {:.2}%, utilization: {:.2}% }}",
            self.total_allocated,
            self.currently_in_use,
            self.peak_usage,
            self.total_allocations,
            self.allocation_failures,
            self.success_rate() * 100.0,
            self.utilization() * 100.0
        )
    }

    /// Reset all statistics
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Thread-safe statistics for buffer pools
#[derive(Debug)]
pub struct AtomicBufferPoolStats {
    /// Total number of buffers ever allocated
    pub total_allocated: AtomicUsize,
    /// Number of buffers currently in use
    pub currently_in_use: AtomicUsize,
    /// Peak number of buffers in use simultaneously
    pub peak_usage: AtomicUsize,
    /// Total number of buffer allocations requested
    pub total_allocations: AtomicU64,
    /// Total number of buffer deallocations
    pub total_deallocations: AtomicU64,
    /// Number of allocation failures
    pub allocation_failures: AtomicU64,
}

impl AtomicBufferPoolStats {
    /// Create new atomic statistics instance
    pub fn new() -> Self {
        Self {
            total_allocated: AtomicUsize::new(0),
            currently_in_use: AtomicUsize::new(0),
            peak_usage: AtomicUsize::new(0),
            total_allocations: AtomicU64::new(0),
            total_deallocations: AtomicU64::new(0),
            allocation_failures: AtomicU64::new(0),
        }
    }

    /// Record a successful allocation
    pub fn record_allocation(&self) {
        self.total_allocations.fetch_add(1, Ordering::Relaxed);
        let new_in_use = self.currently_in_use.fetch_add(1, Ordering::Relaxed) + 1;
        
        // Update peak usage
        let current_peak = self.peak_usage.load(Ordering::Relaxed);
        if new_in_use > current_peak {
            let _ = self.peak_usage.compare_exchange_weak(
                current_peak,
                new_in_use,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }

    /// Record a deallocation
    pub fn record_deallocation(&self) {
        self.total_deallocations.fetch_add(1, Ordering::Relaxed);
        self.currently_in_use.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record an allocation failure
    pub fn record_failure(&self) {
        self.total_allocations.fetch_add(1, Ordering::Relaxed);
        self.allocation_failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Record buffer pool expansion
    pub fn record_expansion(&self, new_buffers: usize) {
        self.total_allocated.fetch_add(new_buffers, Ordering::Relaxed);
    }

    /// Get current statistics snapshot
    pub fn snapshot(&self) -> BufferPoolStats {
        BufferPoolStats {
            total_allocated: self.total_allocated.load(Ordering::Relaxed),
            currently_in_use: self.currently_in_use.load(Ordering::Relaxed),
            peak_usage: self.peak_usage.load(Ordering::Relaxed),
            total_allocations: self.total_allocations.load(Ordering::Relaxed),
            total_deallocations: self.total_deallocations.load(Ordering::Relaxed),
            allocation_failures: self.allocation_failures.load(Ordering::Relaxed),
        }
    }

    /// Reset all statistics
    pub fn reset(&self) {
        self.total_allocated.store(0, Ordering::Relaxed);
        self.currently_in_use.store(0, Ordering::Relaxed);
        self.peak_usage.store(0, Ordering::Relaxed);
        self.total_allocations.store(0, Ordering::Relaxed);
        self.total_deallocations.store(0, Ordering::Relaxed);
        self.allocation_failures.store(0, Ordering::Relaxed);
    }
}

impl Default for AtomicBufferPoolStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper for generating buffer sequence numbers
pub fn next_buffer_sequence() -> u64 {
    static SEQUENCE_COUNTER: std::sync::atomic::AtomicU64 = 
        std::sync::atomic::AtomicU64::new(1);
    SEQUENCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}