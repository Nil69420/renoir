//! Buffer pool statistics tracking

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

/// Helper for generating buffer sequence numbers
pub fn next_buffer_sequence() -> u64 {
    static SEQUENCE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    SEQUENCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}
