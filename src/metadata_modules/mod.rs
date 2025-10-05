//! Metadata management and control regions

pub mod control;
pub mod types;

pub use control::{ControlHeader, ControlRegion};
pub use types::{ControlStats, RegionMetadata, RegionRegistryEntry};
