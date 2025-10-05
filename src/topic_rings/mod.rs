//! Topic-optimized ring buffers for high-performance messaging
//!
//! This module provides specialized ring buffer implementations optimized for
//! topic-based messaging with single and multi producer/consumer patterns.

pub mod mpmc;
pub mod spsc;

// Re-export main types
pub use mpmc::MPMCTopicRing;
pub use spsc::SPSCTopicRing;
