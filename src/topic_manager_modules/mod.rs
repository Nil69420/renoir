//! Topic management system for ROS2-style messaging
//!
//! This module provides a complete topic management system that automatically
//! selects appropriate ring buffer types and shared buffer pools based on
//! topic configuration and usage patterns.

pub mod handles;
pub mod instance;
pub mod manager;
pub mod stats;

pub use handles::{Publisher, Subscriber};
pub use instance::{PublisherHandle, SubscriberHandle, TopicInstance, TopicRingType};
pub use manager::{TopicId, TopicManager};
pub use stats::TopicManagerStats;
