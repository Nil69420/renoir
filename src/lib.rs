//! # Renoir - High-Performance Shared Memory Manager
//!
//! Renoir is a high-performance shared memory manager designed for zero-copy
//! inter-process communication, particularly suited for ROS2 and other
//! real-time systems requiring efficient data sharing.
//!
//! ## Features
//!
//! - **Named shared memory regions**: File-backed and memfd support
//! - **Cross-process topic pub/sub**: Topic rings are placed inside the
//!   shared memory region, so publishers and subscribers in different
//!   processes communicate through the same buffer (see `topic_rings`)
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
pub mod allocators;
pub mod buffers;
pub mod discovery;
pub mod error;
pub mod memory;
pub mod metadata_modules;
pub mod ringbuf;
pub mod rpc;
pub mod sensor;
pub mod shared_pools;
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
pub use allocators::{Allocator, AllocatorExt, BumpAllocator, PoolAllocator};
pub use buffers::{Buffer, BufferPool, BufferPoolConfig, BufferPoolStats};
pub use error::{RenoirError, Result};
pub use memory::SharedMemoryManager;
pub use memory::{BackingType, RegionConfig, RegionMemoryStats, SharedMemoryRegion};
pub use metadata_modules::{
    ControlHeader, ControlRegion, ControlStats, RegionMetadata, RegionRegistryEntry,
};
pub use ringbuf::{ClaimGuard, Consumer, Producer, RingBuffer, SequencedRingBuffer};
pub use shared_pools::{
    BufferHandle, BufferPoolRegistry, PoolId, SharedBufferPool, SharedBufferPoolManager,
};
pub use sync::{
    epoch::{EpochManager, EpochReclaim, ReaderTracker},
    notify::{BatchNotifier, EventNotifier, NotificationGroup, NotifyCondition},
    sequence::{AtomicSequence, GlobalSequenceGenerator, SequenceChecker, SequenceNumber},
    swmr::{ReaderHandle, SWMRRing, SharedSWMRRing, WriterHandle},
    SyncError, SyncResult,
};
pub use topic::{Message, TopicConfig, TopicPattern};
pub use topic_manager_modules::{Publisher, Subscriber, TopicId, TopicManager, TopicManagerStats};
pub use topic_rings::{MPMCTopicRing, SPSCTopicRing};

// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const fn version_component(version: &str, index: usize) -> u32 {
    let mut component = 0u32;
    let mut current = 0usize;
    let mut i = 0usize;
    let bytes = version.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'.' {
            current += 1;
            if current > index {
                break;
            }
        } else if current == index {
            component = component * 10 + (bytes[i] - b'0') as u32;
        }
        i += 1;
    }
    component
}

pub const VERSION_MAJOR: u32 = version_component(VERSION, 0);
pub const VERSION_MINOR: u32 = version_component(VERSION, 1);
pub const VERSION_PATCH: u32 = version_component(VERSION, 2);

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
