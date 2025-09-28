//! Topic management system for ROS2-style messaging
//!
//! This module provides a complete topic management system that automatically
//! selects appropriate ring buffer types and shared buffer pools based on
//! topic configuration and usage patterns.

pub mod manager;
pub mod instance;
pub mod handles;
pub mod stats;

pub use manager::{TopicManager, TopicId};
pub use instance::{TopicInstance, TopicRingType, PublisherHandle, SubscriberHandle};
pub use handles::{Publisher, Subscriber};
pub use stats::TopicManagerStats;