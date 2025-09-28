//! Metadata management and control regions

pub mod types;
pub mod control;

pub use types::{RegionMetadata, RegionRegistryEntry, ControlStats};
pub use control::{ControlHeader, ControlRegion};