//! # Renoir - High-Performance Shared Memory Manager
//!
//! Renoir is a high-performance shared memory manager designed for zero-copy
//! inter-process communication, particularly suited for ROS2 and other
//! real-time systems requiring efficient data sharing.
//!
//! ## Features
//!
//! - **Named shared memory regions**: File-backed and memfd support
//! - **Multiple allocation strategies**: Ring buffers, buffer pools, custom allocators
//! - **Metadata management**: Control regions with sequence/versioning logic
//! - **C API**: Stable interface for C++/ROS2 integration
//! - **Zero-copy operations**: Efficient memory mapping and direct access
//! - **Thread-safe**: Lock-free data structures where possible
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │                Renoir Core                      │
//! ├─────────────────────────────────────────────────┤
//! │  Control/Metadata Region │  Data Regions        │
//! │  - Sequence numbers      │  - Topic buffers     │
//! │  - Version tracking      │  - Buffer pools      │
//! │  - Region registry       │  - Ring buffers      │
//! └─────────────────────────────────────────────────┘
//!           │                         │
//!           ▼                         ▼
//! ┌─────────────────┐    ┌─────────────────────────┐
//! │   C API Layer   │    │     Rust Native API     │
//! │   (C/C++/ROS2)  │    │   (Direct access)       │
//! └─────────────────┘    └─────────────────────────┘
//! ```

pub mod allocator;
pub mod buffer;
pub mod error;
pub mod memory;
pub mod metadata;
pub mod ringbuf;
pub mod topic;
pub mod topic_rings;
pub mod shared_pools;
pub mod topic_manager;
pub mod structured_layout;

#[cfg(feature = "c-api")]
pub mod ffi;

pub use error::{RenoirError, Result};
pub use memory::{SharedMemoryRegion, SharedMemoryManager};
pub use metadata::{ControlRegion, RegionMetadata};
pub use allocator::{Allocator, PoolAllocator, BumpAllocator};
pub use buffer::{BufferPool, Buffer};
pub use ringbuf::{RingBuffer, Producer, Consumer};
pub use topic::{MessageHeader, Message, TopicConfig, TopicPattern, TopicQoS, TopicStats};
pub use topic_rings::{SPSCTopicRing, MPMCTopicRing};
pub use shared_pools::{SharedBufferPoolManager, BufferPoolRegistry};
pub use topic_manager::{TopicManager, Publisher, Subscriber, TopicId};

/// Version information for the Renoir library
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const VERSION_MAJOR: u32 = 0;
pub const VERSION_MINOR: u32 = 1;
pub const VERSION_PATCH: u32 = 0;

/// Default configuration constants
pub mod config {
    /// Default size for control/metadata region (64KB)
    pub const DEFAULT_CONTROL_SIZE: usize = 64 * 1024;
    
    /// Default alignment for memory allocations
    pub const DEFAULT_ALIGNMENT: usize = 64;
    
    /// Maximum number of named regions supported
    pub const MAX_REGIONS: usize = 1024;
    
    /// Default ring buffer capacity
    pub const DEFAULT_RING_CAPACITY: usize = 4096;
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_version_constants() {
        assert!(!VERSION.is_empty());
        assert_eq!(VERSION_MAJOR, 0);
        assert_eq!(VERSION_MINOR, 1);
        assert_eq!(VERSION_PATCH, 0);
    }
}