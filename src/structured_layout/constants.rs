//! Constants and configuration for structured memory layout

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

/// Maximum ring buffer slots per topic
pub const MAX_RING_SLOTS: usize = 1024;

/// Default ring buffer slot size
pub const DEFAULT_SLOT_SIZE: usize = 4096;

/// Buffer pool header magic
pub const BUFFER_POOL_MAGIC: u32 = 0x42504F4F; // "BPOOL"
