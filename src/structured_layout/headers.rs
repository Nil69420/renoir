//! Header structures for structured memory layout

use std::mem::size_of;

use crate::error::{RenoirError, Result};
use super::constants::*;

/// Global control header at the start of each shared memory region
#[repr(C)]
pub struct GlobalControlHeader {
    /// Magic number for validation
    pub magic: u64,
    /// Schema version
    pub version: u32,
    /// Creation timestamp (Unix epoch)
    pub creation_time: u64,
    /// Owner process ID
    pub owner_pid: u32,
    /// Total region size
    pub region_size: u64,
    /// Offset to topic directory
    pub topic_directory_offset: u32,
    /// Maximum number of topics
    pub max_topics: u32,
    /// Current number of topics (atomic for concurrent access)
    pub topic_count: u32,
    /// Global sequence counter (atomic for concurrent access)
    pub global_sequence: u64,
    /// Region configuration flags
    pub config_flags: u32,
    /// Padding to cache line boundary
    _padding: [u8; CACHE_LINE_SIZE - 56],
}

impl GlobalControlHeader {
    /// Create a new global control header
    pub fn new(region_size: u64, max_topics: u32) -> Self {
        Self {
            magic: RENOIR_MAGIC,
            version: SCHEMA_VERSION,
            creation_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap().as_secs(),
            owner_pid: std::process::id(),
            region_size,
            topic_directory_offset: size_of::<GlobalControlHeader>() as u32,
            max_topics,
            topic_count: 0,
            global_sequence: 0,
            config_flags: 0,
            _padding: [0; CACHE_LINE_SIZE - 56],
        }
    }

    /// Validate the header magic and version
    pub fn validate(&self) -> Result<()> {
        if self.magic != RENOIR_MAGIC {
            return Err(RenoirError::invalid_parameter("magic", "Invalid magic number"));
        }
        if self.version != SCHEMA_VERSION {
            return Err(RenoirError::invalid_parameter("version", "Unsupported schema version"));
        }
        Ok(())
    }
}

/// Topic directory entry
#[repr(C)]
pub struct TopicDirectoryEntry {
    /// Topic name (null-terminated)
    pub name: [u8; MAX_TOPIC_NAME_LEN],
    /// Offset to topic control header
    pub topic_offset: u32,
    /// Topic ID
    pub topic_id: u32,
    /// Ring buffer size
    pub ring_size: u32,
    /// Schema ID for type checking
    pub schema_id: u32,
    /// Entry validity flag
    pub valid: u8,
    /// Padding
    _padding: [u8; 3],
}

/// Topic directory with fixed-size entries
#[repr(C)]
pub struct TopicDirectory {
    /// Number of valid entries
    pub entry_count: u32,
    /// Entries bitmap (bit per entry for quick validity check)
    pub validity_bitmap: [u64; (MAX_TOPICS + 63) / 64],
    /// Directory entries
    pub entries: [TopicDirectoryEntry; MAX_TOPICS],
}

impl TopicDirectory {
    /// Create a new empty topic directory
    pub fn new() -> Self {
        const EMPTY_ENTRY: TopicDirectoryEntry = TopicDirectoryEntry {
            name: [0; MAX_TOPIC_NAME_LEN],
            topic_offset: 0,
            topic_id: 0,
            ring_size: 0,
            schema_id: 0,
            valid: 0,
            _padding: [0; 3],
        };
        
        Self {
            entry_count: 0,
            validity_bitmap: [0; (MAX_TOPICS + 63) / 64],
            entries: [EMPTY_ENTRY; MAX_TOPICS],
        }
    }
}

/// Per-topic control header  
#[repr(C)]
pub struct TopicControlHeader {
    /// Topic magic number
    pub magic: u32,
    /// Topic ID
    pub topic_id: u32,
    /// Current sequence counter (use atomics for access)
    pub sequence_counter: u64,
    /// Writer process ID (use atomics for access)
    pub writer_pid: u32,
    /// Last write timestamp (use atomics for access)
    pub last_write_time: u64,
    /// Total messages written (use atomics for access)
    pub messages_written: u64,
    /// Total messages dropped (use atomics for access)
    pub messages_dropped: u64,
    /// Current active readers (use atomics for access)
    pub active_readers: u32,
    /// Ring buffer metadata offset
    pub ring_metadata_offset: u32,
    /// Data area offset
    pub data_area_offset: u32,
    /// Data area size
    pub data_area_size: u32,
    /// Cache line padding
    _padding: [u8; CACHE_LINE_SIZE - 56],
}

/// Ring buffer metadata for SPSC/MPMC operations
#[repr(C)]
pub struct RingMetadata {
    /// Ring buffer capacity
    pub capacity: u32,
    /// Head pointer (use atomics for access)
    pub head: u32,
    /// Tail pointer (use atomics for access) 
    pub tail: u32,
    /// Mask for power-of-2 modulo
    pub mask: u32,
    /// Message slot size
    pub slot_size: u32,
    /// Padding to cache line
    _padding: [u8; CACHE_LINE_SIZE - 20],
}

/// Message slot in the ring buffer
#[repr(C)]
pub struct MessageSlot {
    /// Message sequence number
    pub sequence: u64,
    /// Message size
    pub size: u32,
    /// Message type ID
    pub msg_type: u32,
    /// Timestamp
    pub timestamp: u64,
    // Payload follows immediately after this header
}

/// Shared buffer pool header for large message allocation
#[repr(C)]
pub struct BufferPoolHeader {
    /// Pool magic number
    pub magic: u32,
    /// Chunk size
    pub chunk_size: u32,
    /// Total chunks
    pub total_chunks: u32,
    /// Free chunks (use atomics for access)
    pub free_chunks: u32,
    /// Reference count (use atomics for access)
    pub ref_count: u64,
    /// Free list head (use atomics for access)
    pub free_list_head: u32,
    /// Padding
    _padding: [u8; CACHE_LINE_SIZE - 28],
}