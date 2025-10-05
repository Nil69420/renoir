//! FFI type definitions and handle types

use crate::error::RenoirError;
use std::ffi::{c_char, c_void};

/// Opaque handle types for C API
pub type RenoirManagerHandle = *mut c_void;
pub type RenoirRegionHandle = *mut c_void;
pub type RenoirBufferPoolHandle = *mut c_void;
pub type RenoirBufferHandle = *mut c_void;
pub type RenoirAllocatorHandle = *mut c_void;
pub type RenoirRingBufferHandle = *mut c_void;
pub type RenoirProducerHandle = *mut c_void;
pub type RenoirConsumerHandle = *mut c_void;
pub type RenoirControlRegionHandle = *mut c_void;
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
    pub failure_rate: f64,
    pub is_healthy: bool,
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

/// Control region statistics
#[repr(C)]
pub struct RenoirControlStats {
    pub sequence_number: u64,
    pub total_regions: usize,
    pub active_regions: usize,
    pub stale_regions: usize,
}

/// Region registry entry (C-compatible)
#[repr(C)]
pub struct RenoirRegionRegistryEntry {
    pub name: *const c_char,
    pub size: usize,
    pub backing_type: u32, // 0 = File, 1 = Memfd, 2 = Anonymous
    pub version: u64,
    pub ref_count: u32,
    pub creator_pid: u32,
}

/// Memory statistics structure
#[repr(C)]
pub struct RenoirMemoryStats {
    pub total_allocated: usize,
    pub peak_allocated: usize,
    pub allocation_count: usize,
    pub deallocation_count: usize,
}

/// Topic ID type for C API
pub type RenoirTopicId = u64;

/// Sequence number type
pub type RenoirSequenceNumber = u64;

/// Reserved buffer handle for zero-copy operations
pub type RenoirReservedBufferHandle = *mut c_void;

/// Message handle for received messages
pub type RenoirReceivedMessageHandle = *mut c_void;

/// Topic options for registration
#[repr(C)]
pub struct RenoirTopicOptions {
    pub pattern: u32, // 0 = SPSC, 1 = SPMC, 2 = MPSC, 3 = MPMC
    pub ring_capacity: usize,
    pub max_payload_size: usize,
    pub use_shared_pool: bool,
    pub shared_pool_threshold: usize,
    pub enable_notifications: bool,
}

/// Publisher options
#[repr(C)]
pub struct RenoirPublisherOptions {
    pub batch_size: usize,
    pub timeout_ms: u64, // 0 = no timeout
    pub priority: u32,   // 0 = normal, 1 = high
    pub enable_batching: bool,
}

/// Subscriber options and modes
#[repr(C)]
pub struct RenoirSubscriberOptions {
    pub mode: u32, // 0 = blocking, 1 = non-blocking, 2 = polling
    pub batch_size: usize,
    pub timeout_ms: u64, // 0 = no timeout
    pub queue_depth: usize,
    pub enable_filtering: bool,
}

/// Message metadata structure
#[repr(C)]
pub struct RenoirMessageMetadata {
    pub timestamp_ns: u64,
    pub sequence_number: RenoirSequenceNumber,
    pub source_id: u32,
    pub message_type: u32,
    pub flags: u32,
}

/// Received message structure
#[repr(C)]
pub struct RenoirReceivedMessage {
    pub payload_ptr: *const u8,
    pub payload_len: usize,
    pub metadata: RenoirMessageMetadata,
    pub handle: RenoirReceivedMessageHandle,
}

/// Topic statistics
#[repr(C)]
pub struct RenoirTopicStats {
    pub topic_id: RenoirTopicId,
    pub publisher_count: usize,
    pub subscriber_count: usize,
    pub messages_published: u64,
    pub messages_consumed: u64,
    pub messages_dropped: u64,
    pub bytes_transferred: u64,
    pub average_latency_ns: u64,
    pub peak_latency_ns: u64,
    pub throughput_mbps: f64,
}

/// Region statistics
#[repr(C)]
pub struct RenoirRegionStats {
    pub total_size: usize,
    pub used_size: usize,
    pub free_size: usize,
    pub fragmentation_ratio: f64,
    pub topic_count: usize,
    pub allocation_count: u64,
}

/// Topic info for listing
#[repr(C)]
pub struct RenoirTopicInfo {
    pub topic_id: RenoirTopicId,
    pub name: *const c_char,
    pub pattern: u32,
    pub publisher_count: usize,
    pub subscriber_count: usize,
    pub created_timestamp_ns: u64,
}

/// Error information structure
#[repr(C)]
pub struct RenoirErrorInfo {
    pub code: RenoirErrorCode,
    pub message: *const c_char,
    pub source_file: *const c_char,
    pub source_line: u32,
}
