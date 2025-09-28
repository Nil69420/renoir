//! Topic-based messaging system for ROS2 integration
//! 
//! This module provides per-topic data structures optimized for different usage patterns:
//! - SPSC rings for single sensor writer → many readers
//! - MPMC rings for multiple writers/readers  
//! - Shared buffer pools for large objects referenced by descriptors

pub mod header;
pub mod config;
pub mod message;
pub mod stats;

#[cfg(test)]
mod tests;

// Re-export main types for convenience
pub use header::{MessageHeader, MESSAGE_MAGIC, MESSAGE_VERSION, MAX_TOPIC_NAME_LENGTH};
pub use config::{TopicPattern, TopicConfig, TopicQoS, Reliability};
pub use message::{MessageDescriptor, MessagePayload, Message};
pub use stats::TopicStats;