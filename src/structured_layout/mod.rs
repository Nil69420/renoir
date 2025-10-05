//! Structured memory layout implementation following recommended schema patterns
//!
//! This module implements a hierarchical shared memory layout:
//! 1. Global control header (magic, version, metadata)
//! 2. Topic directory (name → offset mapping)
//! 3. Per-topic blocks (control + ring + data)
//! 4. Shared buffer pools (optional, for large messages)

pub mod constants;
pub mod headers;
pub mod region;

// Re-export main types
pub use constants::*;
pub use headers::{
    BufferPoolHeader, GlobalControlHeader, MessageSlot, RingMetadata, TopicControlHeader,
    TopicDirectory, TopicDirectoryEntry,
};
pub use region::{GlobalStats, StructuredMemoryRegion};
