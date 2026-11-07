//! Topic-optimized ring buffers for high-performance messaging
//!
//! This module provides specialized ring buffer implementations optimized for
//! topic-based messaging with single and multi producer/consumer patterns.

pub mod broadcast;
pub mod mpmc;
pub mod spsc;

// Re-export main types
pub use broadcast::{
    BroadcastCursor, BroadcastReservation, BroadcastTopicRing, ReadInto, BROADCAST_RING_MAGIC,
};
pub use mpmc::{MPMCTopicRing, MPMC_RING_MAGIC};
pub use spsc::{
    RingControlInfo, SPSCTopicRing, SESSION_CLEAN, SESSION_DIRTY, SESSION_MARKER_OFFSET,
    SPSC_RING_MAGIC,
};
