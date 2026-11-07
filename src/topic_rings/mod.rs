//! Topic-optimized ring buffers for high-performance messaging
//!
//! This module provides specialized ring buffer implementations optimized for
//! topic-based messaging with single and multi producer/consumer patterns.

pub(crate) mod atomic_bytes;
pub mod broadcast;
pub(crate) mod futex;
pub mod mpmc;
pub mod spsc;

// Re-export main types
pub use broadcast::{
    BroadcastControlInfo, BroadcastCursor, BroadcastReservation, BroadcastTopicRing, ReadInto,
    BROADCAST_HEADER_PROBE_BYTES, BROADCAST_RING_MAGIC,
};
pub use mpmc::{MPMCTopicRing, MPMC_RING_MAGIC};
pub use spsc::{
    RingControlInfo, SPSCTopicRing, SESSION_CLEAN, SESSION_DIRTY, SESSION_MARKER_OFFSET,
    SPSC_RING_MAGIC,
};
