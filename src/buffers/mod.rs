//! Buffer management and memory pools
//!
//! This module provides efficient buffer allocation and management for
//! high-performance messaging and data transfer.

pub mod buffer;
pub mod config;
pub mod pool;
pub mod stats;

// Re-export main types  
pub use buffer::Buffer;
pub use config::{BufferPoolConfig, BufferPoolConfigBuilder};
pub use pool::BufferPool;
pub use stats::{BufferPoolStats, AtomicBufferPoolStats, next_buffer_sequence};