//! C Foreign Function Interface (FFI) for C++/ROS2 integration
//!
//! This module provides a complete C-compatible API for Renoir shared memory library,
//! organized into logical submodules for better maintainability.

pub mod types;
pub mod utils;
pub mod memory;
pub mod buffers;
pub mod topics;
pub mod version;

// Re-export commonly used types and functions
pub use types::{
    RenoirErrorCode, RenoirManagerHandle, RenoirRegionHandle, RenoirBufferPoolHandle,
    RenoirBufferHandle, RenoirTopicManagerHandle, RenoirPublisherHandle, RenoirSubscriberHandle,
    RenoirRegionConfig, RenoirBufferPoolConfig, RenoirBufferInfo
};

pub use utils::{renoir_free_string, HANDLE_REGISTRY};
pub use memory::{
    renoir_manager_create, renoir_manager_create_with_control, renoir_manager_destroy,
    renoir_region_create, renoir_region_get, renoir_region_size, renoir_region_name
};
pub use buffers::{
    renoir_buffer_pool_create, renoir_buffer_get, renoir_buffer_info,
    renoir_buffer_return, renoir_buffer_pool_stats, renoir_buffer_pool_destroy
};
pub use topics::{
    renoir_topic_manager_create, renoir_topic_manager_destroy, renoir_topic_create,
    renoir_publisher_create, renoir_subscriber_create, renoir_publish_message,
    renoir_receive_message, renoir_publisher_destroy, renoir_subscriber_destroy,
    renoir_topic_remove
};
pub use version::{
    renoir_version_major, renoir_version_minor, renoir_version_patch, renoir_version_string
};