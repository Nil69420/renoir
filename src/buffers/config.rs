//! Buffer pool configuration

use std::time::Duration;

/// Configuration for buffer pools
#[derive(Debug, Clone, PartialEq)]
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

impl BufferPoolConfig {
    /// Create a new configuration with custom name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set buffer size
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Set initial buffer count
    pub fn with_initial_count(mut self, count: usize) -> Self {
        self.initial_count = count;
        self
    }

    /// Set maximum buffer count
    pub fn with_max_count(mut self, count: usize) -> Self {
        self.max_count = count;
        self
    }

    /// Set buffer alignment
    pub fn with_alignment(mut self, alignment: usize) -> Self {
        self.alignment = alignment;
        self
    }

    /// Set pre-allocation behavior
    pub fn with_pre_allocate(mut self, pre_allocate: bool) -> Self {
        self.pre_allocate = pre_allocate;
        self
    }

    /// Set allocation timeout
    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.allocation_timeout = timeout;
        self
    }

    /// Validate the configuration
    pub fn validate(&self) -> crate::error::Result<()> {
        use crate::error::RenoirError;

        if self.buffer_size == 0 {
            return Err(RenoirError::InvalidParameter {
                parameter: "buffer_size".to_string(),
                message: "Buffer size cannot be zero".to_string(),
            });
        }

        if self.max_count == 0 {
            return Err(RenoirError::InvalidParameter {
                parameter: "max_count".to_string(),
                message: "Max count cannot be zero".to_string(),
            });
        }

        if self.initial_count > self.max_count {
            return Err(RenoirError::InvalidParameter {
                parameter: "initial_count".to_string(),
                message: "Initial count cannot exceed max count".to_string(),
            });
        }

        if !self.alignment.is_power_of_two() {
            return Err(RenoirError::InvalidParameter {
                parameter: "alignment".to_string(),
                message: "Alignment must be a power of two".to_string(),
            });
        }

        Ok(())
    }

    /// Calculate total memory required
    pub fn total_memory_required(&self) -> usize {
        self.buffer_size * self.max_count
    }

    /// Get memory overhead per buffer (for metadata, alignment, etc.)
    pub fn memory_overhead(&self) -> usize {
        // Account for alignment padding and metadata
        let aligned_size = (self.buffer_size + self.alignment - 1) & !(self.alignment - 1);
        aligned_size - self.buffer_size + std::mem::size_of::<usize>() // metadata size
    }
}

/// Builder pattern for buffer pool configuration
pub struct BufferPoolConfigBuilder {
    config: BufferPoolConfig,
}

impl BufferPoolConfigBuilder {
    /// Create a new builder
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            config: BufferPoolConfig::new(name),
        }
    }

    /// Set buffer size
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.config.buffer_size = size;
        self
    }

    /// Set initial count
    pub fn initial_count(mut self, count: usize) -> Self {
        self.config.initial_count = count;
        self
    }

    /// Set maximum count
    pub fn max_count(mut self, count: usize) -> Self {
        self.config.max_count = count;
        self
    }

    /// Set alignment
    pub fn alignment(mut self, alignment: usize) -> Self {
        self.config.alignment = alignment;
        self
    }

    /// Enable or disable pre-allocation
    pub fn pre_allocate(mut self, enable: bool) -> Self {
        self.config.pre_allocate = enable;
        self
    }

    /// Set timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.allocation_timeout = Some(timeout);
        self
    }

    /// No timeout
    pub fn no_timeout(mut self) -> Self {
        self.config.allocation_timeout = None;
        self
    }

    /// Build the configuration
    pub fn build(self) -> crate::error::Result<BufferPoolConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}