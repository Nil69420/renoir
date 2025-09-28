//! Structured memory layout implementation following recommended schema patterns
//! 
//! This module implements a hierarchical shared memory layout:
//! 1. Global control header (magic, version, metadata)
//! 2. Topic directory (name → offset mapping)
//! 3. Per-topic blocks (control + ring + data)
//! 4. Shared buffer pools (optional, for large messages)

use std::{
    mem::size_of,
    sync::Arc,
};

use crate::{
    error::{RenoirError, Result},
    memory::SharedMemoryRegion,
};

/// Cache line size for alignment (64 bytes on most x86_64 systems)
pub const CACHE_LINE_SIZE: usize = 64;

/// Magic number to identify shared memory regions
pub const RENOIR_MAGIC: u64 = 0x52454E4F49525348; // "RENOIRSH"

/// Current schema version
pub const SCHEMA_VERSION: u32 = 1;

/// Maximum number of topics per region  
pub const MAX_TOPICS: usize = 1024;

/// Maximum topic name length
pub const MAX_TOPIC_NAME_LEN: usize = 64;

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

/// Main structured memory region manager
pub struct StructuredMemoryRegion {
    region: Arc<SharedMemoryRegion>,
    global_header: *mut GlobalControlHeader,
    topic_directory: *mut TopicDirectory,
}

unsafe impl Send for StructuredMemoryRegion {}
unsafe impl Sync for StructuredMemoryRegion {}

impl StructuredMemoryRegion {
    /// Create or attach to a structured memory region
    pub fn create(region: Arc<SharedMemoryRegion>, initialize: bool) -> Result<Self> {
        let base_ptr: *mut u8 = unsafe { region.as_mut_ptr_unsafe() };
        let global_header = base_ptr as *mut GlobalControlHeader;
        
        if initialize {
            // Initialize the global header
            let header = GlobalControlHeader::new(region.size() as u64, MAX_TOPICS as u32);
            unsafe {
                std::ptr::write(global_header, header);
            }
            
            // Initialize the topic directory
            let topic_directory = unsafe { 
                base_ptr.add(size_of::<GlobalControlHeader>()) as *mut TopicDirectory 
            };
            unsafe {
                std::ptr::write(topic_directory, TopicDirectory::new());
            }
        } else {
            // Validate existing region
            unsafe {
                (*global_header).validate()?;
            }
        }
        
        let topic_directory = unsafe {
            base_ptr.add((*global_header).topic_directory_offset as usize) as *mut TopicDirectory
        };
        
        Ok(Self {
            region,
            global_header,
            topic_directory,
        })
    }

    /// Find a topic by name
    pub fn find_topic(&self, name: &str) -> Option<&TopicDirectoryEntry> {
        let name_bytes = name.as_bytes();
        if name_bytes.len() >= MAX_TOPIC_NAME_LEN {
            return None;
        }
        
        let directory = unsafe { &*self.topic_directory };
        
        for i in 0..directory.entry_count as usize {
            let entry = &directory.entries[i];
            if entry.valid == 0 {
                continue;
            }
            
            let entry_name_len = entry.name.iter()
                .position(|&x| x == 0)
                .unwrap_or(MAX_TOPIC_NAME_LEN);
            
            if entry_name_len == name_bytes.len() && 
               &entry.name[..entry_name_len] == name_bytes {
                return Some(entry);
            }
        }
        
        None
    }

    /// Create a new topic
    pub fn create_topic(&mut self, name: &str, ring_size: u32, schema_id: u32) -> Result<u32> {
        let name_bytes = name.as_bytes();
        if name_bytes.len() >= MAX_TOPIC_NAME_LEN {
            return Err(RenoirError::invalid_parameter("name", "Topic name too long"));
        }
        
        // Check if topic already exists
        if self.find_topic(name).is_some() {
            return Err(RenoirError::invalid_parameter("name", "Topic already exists"));
        }
        
        let global_header = unsafe { &mut *self.global_header };
        let topic_directory = unsafe { &mut *self.topic_directory };
        
        if topic_directory.entry_count >= MAX_TOPICS as u32 {
            return Err(RenoirError::invalid_parameter("topic_count", "Too many topics"));
        }
        
        // Generate new topic ID
        let topic_id = global_header.global_sequence + 1;
        global_header.global_sequence = topic_id;
        
        // Find free entry slot
        let entry_index = topic_directory.entry_count as usize;
        let entry = &mut topic_directory.entries[entry_index];
        
        // Initialize entry
        entry.name[..name_bytes.len()].copy_from_slice(name_bytes);
        entry.name[name_bytes.len()..].fill(0);
        entry.topic_id = topic_id as u32;
        entry.ring_size = ring_size;
        entry.schema_id = schema_id;
        entry.valid = 1;
        
        // Update directory metadata
        topic_directory.entry_count += 1;
        global_header.topic_count += 1;
        
        // Set validity bit
        let bit_index = entry_index;
        let word_index = bit_index / 64;
        let bit_offset = bit_index % 64;
        topic_directory.validity_bitmap[word_index] |= 1u64 << bit_offset;
        
        Ok(topic_id as u32)
    }

    /// Get global statistics
    pub fn get_global_stats(&self) -> GlobalStats {
        let header = unsafe { &*self.global_header };
        GlobalStats {
            magic: header.magic,
            version: header.version,
            creation_time: header.creation_time,
            owner_pid: header.owner_pid,
            region_size: header.region_size,
            max_topics: header.max_topics,
            topic_count: header.topic_count,
            global_sequence: header.global_sequence,
        }
    }

    /// Get topic control header for a topic
    pub fn get_topic_header(&self, topic_offset: u32) -> Result<*mut TopicControlHeader> {
        let base_ptr: *mut u8 = unsafe { self.region.as_mut_ptr_unsafe() };
        let topic_header = unsafe {
            base_ptr.add(topic_offset as usize) as *mut TopicControlHeader
        };
        Ok(topic_header)
    }
}

/// Global statistics structure
#[derive(Debug, Clone)]
pub struct GlobalStats {
    pub magic: u64,
    pub version: u32,
    pub creation_time: u64,
    pub owner_pid: u32,
    pub region_size: u64,
    pub max_topics: u32,
    pub topic_count: u32,
    pub global_sequence: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SharedMemoryManager;

    #[test]
    fn test_structured_layout_basic() {
        let manager = SharedMemoryManager::new();
        let config = crate::memory::RegionConfig {
            name: "test_structured".to_string(),
            size: 1024 * 1024,
            backing_type: crate::memory::BackingType::MemFd,
            file_path: None,
            create: true,
            permissions: 0o600,
        };
        let region = manager.create_region(config).unwrap();
        
        let mut structured = StructuredMemoryRegion::create(region, true).unwrap();
        
        // Test global header validation
        let stats = structured.get_global_stats();
        assert_eq!(stats.magic, RENOIR_MAGIC);
        assert_eq!(stats.version, SCHEMA_VERSION);
        assert_eq!(stats.topic_count, 0);
        
        // Test topic creation
        let topic_id = structured.create_topic("test_topic", 1024, 1).unwrap();
        assert!(topic_id > 0);
        
        // Test topic lookup
        let entry = structured.find_topic("test_topic").unwrap();
        assert_eq!(entry.topic_id, topic_id);
        assert_eq!(entry.ring_size, 1024);
        assert_eq!(entry.schema_id, 1);
        
        let stats = structured.get_global_stats();
        assert_eq!(stats.topic_count, 1);
    }
}