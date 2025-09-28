//! FFI type definitions and handle types

use std::ffi::{c_char, c_void};
use crate::error::RenoirError;

/// Opaque handle types for C API
pub type RenoirManagerHandle = *mut c_void;
pub type RenoirRegionHandle = *mut c_void;
pub type RenoirBufferPoolHandle = *mut c_void;
pub type RenoirBufferHandle = *mut c_void;
pub type RenoirAllocatorHandle = *mut c_void;
pub type RenoirRingBufferHandle = *mut c_void;
pub type RenoirProducerHandle = *mut c_void;
pub type RenoirConsumerHandle = *mut c_void;
pub type RenoirTopicManagerHandle = *mut c_void;
pub type RenoirPublisherHandle = *mut c_void;
pub type RenoirSubscriberHandle = *mut c_void;
pub type RenoirMessageHandle = *mut c_void;

/// Error codes for C API
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenoirErrorCode {
    Success = 0,
    InvalidParameter = 1,
    OutOfMemory = 2,
    RegionNotFound = 3,
    RegionExists = 4,
    InsufficientSpace = 5,
    BufferFull = 6,
    BufferEmpty = 7,
    IoError = 8,
    SerializationError = 9,
    ConcurrencyError = 10,
    AlignmentError = 11,
    VersionMismatch = 12,
    PlatformError = 13,
    UnknownError = 99,
}

impl From<RenoirError> for RenoirErrorCode {
    fn from(error: RenoirError) -> Self {
        match error {
            RenoirError::InvalidParameter { .. } => RenoirErrorCode::InvalidParameter,
            RenoirError::Memory { .. } => RenoirErrorCode::OutOfMemory,
            RenoirError::RegionNotFound { .. } => RenoirErrorCode::RegionNotFound,
            RenoirError::RegionExists { .. } => RenoirErrorCode::RegionExists,
            RenoirError::InsufficientSpace { .. } => RenoirErrorCode::InsufficientSpace,
            RenoirError::BufferFull { .. } => RenoirErrorCode::BufferFull,
            RenoirError::BufferEmpty { .. } => RenoirErrorCode::BufferEmpty,
            RenoirError::Io { .. } => RenoirErrorCode::IoError,
            RenoirError::Serialization { .. } => RenoirErrorCode::SerializationError,
            RenoirError::Concurrency { .. } => RenoirErrorCode::ConcurrencyError,
            RenoirError::Alignment { .. } => RenoirErrorCode::AlignmentError,
            RenoirError::VersionMismatch { .. } => RenoirErrorCode::VersionMismatch,
            RenoirError::Platform { .. } => RenoirErrorCode::PlatformError,
        }
    }
}

/// Configuration structure for creating regions (C-compatible)
#[repr(C)]
pub struct RenoirRegionConfig {
    pub name: *const c_char,
    pub size: usize,
    pub backing_type: u32, // 0 = FileBacked, 1 = MemFd
    pub file_path: *const c_char,
    pub create: bool,
    pub permissions: u32,
}

/// Buffer pool configuration (C-compatible)
#[repr(C)]
pub struct RenoirBufferPoolConfig {
    pub name: *const c_char,
    pub buffer_size: usize,
    pub initial_count: usize,
    pub max_count: usize,
    pub alignment: usize,
    pub pre_allocate: bool,
    pub allocation_timeout_ms: u64, // 0 = no timeout
}

/// Buffer information structure
#[repr(C)]
pub struct RenoirBufferInfo {
    pub size: usize,
    pub capacity: usize,
    pub alignment: usize,
    pub pool_name: *const c_char,
}

/// Buffer pool statistics (C-compatible)
#[repr(C)]
pub struct RenoirBufferPoolStats {
    pub total_allocated: usize,
    pub currently_in_use: usize,
    pub peak_usage: usize,
    pub total_allocations: u64,
    pub total_deallocations: u64,
    pub allocation_failures: u64,
    pub success_rate: f64,
    pub utilization: f64,
}

/// Ring buffer statistics
#[repr(C)]
pub struct RenoirRingStats {
    pub capacity: usize,
    pub size: usize,
    pub writes: u64,
    pub reads: u64,
    pub overwrites: u64,
}

/// Memory statistics structure
#[repr(C)]
pub struct RenoirMemoryStats {
    pub total_allocated: usize,
    pub peak_allocated: usize,
    pub allocation_count: usize,
    pub deallocation_count: usize,
}