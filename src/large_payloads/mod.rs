//! Large payload handling for variable-sized messages (images, point clouds)
//!
//! This module extends the existing shared buffer pool system with specialized
//! handling for ROS2 message types that can have highly variable sizes.
//!
//! Key components:
//! - Blob headers with magic numbers, versioning, and checksums for data integrity
//! - Chunking system for payloads exceeding buffer pool limits
//! - Epoch-based reclamation with per-reader watermarks
//! - ROS2-specific message structures and content type handlers

pub mod blob;
pub mod chunking;
pub mod reclamation;
pub mod ros2_types;

// Re-export main types for convenience
pub use blob::{content_types, BlobDescriptor, BlobHeader, BlobManager};
pub use chunking::{ChunkDescriptor, ChunkManager, ChunkingStrategy};
pub use reclamation::{EpochReclaimer, ReclamationPolicy, ReclamationStats};
pub use ros2_types::{
    ImageHeader, ImageMessage, LaserScanHeader, LaserScanMessage, PointCloudHeader,
    PointCloudMessage, ROS2MessageManager, ROS2MessageType,
};
