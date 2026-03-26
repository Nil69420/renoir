//! Shared memory region management and operations

pub mod config;
pub mod manager;
pub mod regions;

pub use config::{BackingType, RegionConfig};
pub use manager::SharedMemoryManager;
pub use regions::{RegionMemoryStats, SharedMemoryRegion};
