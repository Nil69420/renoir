//! Topic-based messaging system for ROS2 integration
//!
//! This module provides per-topic data structures optimized for different usage patterns:
//! - SPSC rings for single sensor writer → many readers
//! - MPMC rings for multiple writers/readers  
//! - Shared buffer pools for large objects referenced by descriptors

pub mod config;
pub mod header;
pub mod message;
pub mod stats;

#[cfg(test)]
mod tests;

// Re-export main types for convenience
pub use config::{Reliability, TopicConfig, TopicPattern, TopicQoS};
pub use header::{MessageHeader, MAX_TOPIC_NAME_LENGTH, MESSAGE_MAGIC, MESSAGE_VERSION};
pub use message::{Message, MessageDescriptor, MessagePayload};
pub use stats::TopicStats;
