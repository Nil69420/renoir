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
//! - **Modular architecture**: Clean separation of concerns, all files under 300 lines
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

// All code should be actively used in production

// Core modules
pub mod error;
pub mod memory;
pub mod allocators;
pub mod buffers;
pub mod metadata_modules;
pub mod ringbuf;
pub mod shared_pools;
pub mod structured_layout;
pub mod topic;
pub mod topic_manager_modules;
pub mod topic_rings;

// Advanced features
pub mod large_payloads;

// New message format and schema strategy module
pub mod message_formats;

// Synchronization primitives for high-performance robotics patterns
pub mod sync;

#[cfg(feature = "c-api")]
pub mod ffi;

// Main API re-exports
pub use memory::SharedMemoryManager;
pub use topic_manager_modules::{TopicManager, Publisher, Subscriber, TopicId, TopicManagerStats};
pub use topic::{TopicConfig, TopicPattern, Message};
pub use error::{RenoirError, Result};
pub use allocators::{Allocator, AllocatorExt, BumpAllocator, PoolAllocator};
pub use buffers::{Buffer, BufferPool, BufferPoolConfig, BufferPoolConfigBuilder, BufferPoolStats, AtomicBufferPoolStats};
pub use shared_pools::{SharedBufferPoolManager, SharedBufferPool, BufferPoolRegistry, PoolId, BufferHandle};
pub use memory::{BackingType, RegionConfig, SharedMemoryRegion, RegionMemoryStats};
pub use metadata_modules::{RegionMetadata, RegionRegistryEntry, ControlStats, ControlHeader, ControlRegion};
pub use structured_layout::{StructuredMemoryRegion, GlobalStats, GlobalControlHeader, TopicDirectory};
pub use topic_rings::{SPSCTopicRing, MPMCTopicRing};
pub use ringbuf::{RingBuffer, Producer, Consumer, SequencedRingBuffer, ClaimGuard};
pub use sync::{
    sequence::{AtomicSequence, SequenceChecker, GlobalSequenceGenerator, SequenceNumber},
    swmr::{SWMRRing, SharedSWMRRing, WriterHandle, ReaderHandle},
    epoch::{EpochManager, SharedEpochManager, EpochReclaim, ReaderTracker},
    notify::{EventNotifier, NotificationGroup, BatchNotifier, NotifyCondition},
    SyncError, SyncResult
};

// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const VERSION_MAJOR: u32 = 0;
pub const VERSION_MINOR: u32 = 3;
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