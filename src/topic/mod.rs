//! Topic-based messaging system for ROS2 integration
//!
//! This module provides per-topic data structures optimized for different usage patterns:
//! - SPSC rings for single writer → single reader
//! - MPMC rings for multiple writers/readers
//! - Shared buffer pools for large objects referenced by descriptors
//!
//! Ring buffers are placed inside the shared memory region allocated for the
//! topic (`TopicInstance`), so publishers and subscribers running in separate
//! processes attached to the same region communicate through the same
//! buffer.

pub mod config;
pub mod header;
pub mod message;
pub mod stats;

// Re-export main types for convenience
pub use config::{Reliability, TopicConfig, TopicPattern, TopicQoS};
pub use header::{
    MessageHeader, FORMAT_TYPE_DESCRIPTOR, FORMAT_TYPE_INLINE, MAX_TOPIC_NAME_LENGTH,
    MESSAGE_MAGIC, MESSAGE_VERSION,
};
pub use message::{Message, MessageDescriptor, MessagePayload};
pub use stats::TopicStats;
