//! Shared buffer pool system for large message payloads
//!
//! This module provides descriptor-based storage where ring buffers contain
//! small descriptors pointing to actual large data stored in shared buffer pools.

pub mod manager;
pub mod pool;
pub mod registry;
pub mod stats;

pub use manager::{PoolId, SharedBufferPoolManager};
pub use pool::{BufferHandle, SharedBufferPool};
pub use registry::BufferPoolRegistry;
pub use stats::{SharedPoolBufferStats, SharedPoolStats};
