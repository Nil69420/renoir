//! Shared buffer pool system for large message payloads
//!
//! This module provides descriptor-based storage where ring buffers contain
//! small descriptors pointing to actual large data stored in shared buffer pools.

pub mod manager;
pub mod pool;
pub mod stats;
pub mod registry;

pub use manager::{SharedBufferPoolManager, PoolId};
pub use pool::{SharedBufferPool, BufferHandle};
pub use stats::{SharedPoolStats, SharedPoolBufferStats};
pub use registry::BufferPoolRegistry;