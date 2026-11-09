//! Topic-optimized ring buffers for high-performance messaging
//!
//! This module provides specialized ring buffer implementations optimized for
//! topic-based messaging with single and multi producer/consumer patterns.

pub(crate) mod atomic_bytes;
pub mod broadcast;
pub(crate) mod futex;
mod header_bytes;
mod init_guard;
pub mod lean;
pub mod mpmc;
pub mod spsc;

// Re-export main types
pub use broadcast::{
    BroadcastControlInfo, BroadcastCursor, BroadcastReservation, BroadcastTopicRing, BroadcastView,
    ReadInto, ViewRead, BROADCAST_HEADER_PROBE_BYTES, BROADCAST_RING_MAGIC,
};
pub use lean::{
    frame_slot_bytes, inspect_lean_header, lean_slot_bytes, DeadlineWait, FrameChannel, FrameLoan,
    FrameView, LeanChannel, LeanCursor, LeanGatedCursor, LeanInfo, LeanMode, LeanPublisher,
    LeanQos, LeanRead, LeanReliability, LeanRing, LEAN_HEADER_BYTES, LEAN_MAX_CONSUMERS,
    LEAN_MAX_FRAME_BYTES, LEAN_MAX_SLOT_LINES, LEAN_RING_MAGIC,
};
pub use mpmc::{MPMCTopicRing, MPMC_RING_MAGIC};
pub use spsc::{
    RingControlInfo, SPSCTopicRing, SESSION_CLEAN, SESSION_DIRTY, SESSION_MARKER_OFFSET,
    SPSC_RING_MAGIC,
};
