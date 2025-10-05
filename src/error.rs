//! Error types and handling for Renoir

/// Result type alias for Renoir operations
pub type Result<T> = std::result::Result<T, RenoirError>;

/// Comprehensive error types for the Renoir shared memory manager
#[derive(Debug, thiserror::Error)]
pub enum RenoirError {
    /// I/O related errors (file operations, mmap, etc.)
    #[error("I/O error: {message}")]
    Io {
        message: String,
        #[source]
        source: Option<std::io::Error>,
    },

    /// Memory allocation or mapping failures
    #[error("Memory error: {message}")]
    Memory { message: String },

    /// Invalid parameters or configuration
    #[error("Invalid parameter: {parameter} - {message}")]
    InvalidParameter { parameter: String, message: String },

    /// Region not found or doesn't exist
    #[error("Region not found: {name}")]
    RegionNotFound { name: String },

    /// Region already exists
    #[error("Region already exists: {name}")]
    RegionExists { name: String },

    /// Insufficient space for allocation
    #[error("Insufficient space: requested {requested}, available {available}")]
    InsufficientSpace { requested: usize, available: usize },

    /// Alignment requirements not met
    #[error("Alignment error: address {address:#x} not aligned to {alignment}")]
    Alignment { address: usize, alignment: usize },

    /// Version or sequence number mismatch
    #[error("Version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: u64, actual: u64 },

    /// Concurrency related errors
    #[error("Concurrency error: {message}")]
    Concurrency { message: String },

    /// Buffer is full (for ring buffers, pools, etc.)
    #[error("Buffer full: {buffer_type}")]
    BufferFull { buffer_type: String },

    /// Buffer is empty
    #[error("Buffer empty: {buffer_type}")]
    BufferEmpty { buffer_type: String },

    /// Serialization/deserialization errors
    #[error("Serialization error: {message}")]
    Serialization { message: String },

    /// Platform-specific errors
    #[error("Platform error: {message}")]
    Platform { message: String },
}

impl RenoirError {
    /// Create an I/O error from a standard I/O error
    pub fn from_io(source: std::io::Error, context: &str) -> Self {
        Self::Io {
            message: format!("{}: {}", context, source),
            source: Some(source),
        }
    }

    /// Create a memory error
    pub fn memory(message: impl Into<String>) -> Self {
        Self::Memory {
            message: message.into(),
        }
    }

    /// Create an invalid parameter error
    pub fn invalid_parameter(parameter: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidParameter {
            parameter: parameter.into(),
            message: message.into(),
        }
    }

    /// Create a region not found error
    pub fn region_not_found(name: impl Into<String>) -> Self {
        Self::RegionNotFound { name: name.into() }
    }

    /// Create a region exists error
    pub fn region_exists(name: impl Into<String>) -> Self {
        Self::RegionExists { name: name.into() }
    }

    /// Create an insufficient space error
    pub fn insufficient_space(requested: usize, available: usize) -> Self {
        Self::InsufficientSpace {
            requested,
            available,
        }
    }

    /// Create an alignment error
    pub fn alignment(address: usize, alignment: usize) -> Self {
        Self::Alignment { address, alignment }
    }

    /// Create a version mismatch error
    pub fn version_mismatch(expected: u64, actual: u64) -> Self {
        Self::VersionMismatch { expected, actual }
    }

    /// Create a concurrency error
    pub fn concurrency(message: impl Into<String>) -> Self {
        Self::Concurrency {
            message: message.into(),
        }
    }

    /// Create a buffer full error
    pub fn buffer_full(buffer_type: impl Into<String>) -> Self {
        Self::BufferFull {
            buffer_type: buffer_type.into(),
        }
    }

    /// Create a buffer empty error
    pub fn buffer_empty(buffer_type: impl Into<String>) -> Self {
        Self::BufferEmpty {
            buffer_type: buffer_type.into(),
        }
    }

    /// Create a serialization error
    pub fn serialization(message: impl Into<String>) -> Self {
        Self::Serialization {
            message: message.into(),
        }
    }

    /// Create a platform error
    pub fn platform(message: impl Into<String>) -> Self {
        Self::Platform {
            message: message.into(),
        }
    }
}

// Convert from common error types
impl From<std::io::Error> for RenoirError {
    fn from(err: std::io::Error) -> Self {
        Self::from_io(err, "I/O operation failed")
    }
}

impl From<bincode::Error> for RenoirError {
    fn from(err: bincode::Error) -> Self {
        Self::serialization(format!("Bincode error: {}", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = RenoirError::memory("Out of memory");
        assert!(matches!(err, RenoirError::Memory { .. }));

        let err = RenoirError::region_not_found("test_region");
        assert!(matches!(err, RenoirError::RegionNotFound { .. }));

        let err = RenoirError::insufficient_space(1024, 512);
        assert!(matches!(err, RenoirError::InsufficientSpace { .. }));
    }

    #[test]
    fn test_error_display() {
        let err = RenoirError::memory("Test message");
        let display = format!("{}", err);
        assert!(display.contains("Memory error"));
        assert!(display.contains("Test message"));
    }
}
