//! Shared memory region management and operations

pub mod config;
pub mod regions;
pub mod manager;

pub use config::{BackingType, RegionConfig};
pub use regions::{SharedMemoryRegion, RegionMemoryStats};
pub use manager::SharedMemoryManager;