//! C Foreign Function Interface (FFI) for C++/ROS2 integration
//!
//! This module provides a complete C-compatible API for Renoir shared memory library,
//! organized into logical submodules for better maintainability.

pub mod buffers;
pub mod memory;
pub mod topics;
pub mod types;
pub mod utils;
pub mod version;

// Re-export commonly used types and functions
pub use types::{
    RenoirBufferHandle, RenoirBufferInfo, RenoirBufferPoolConfig, RenoirBufferPoolHandle,
    RenoirErrorCode, RenoirErrorInfo, RenoirManagerHandle, RenoirMessageMetadata,
    RenoirPublisherHandle, RenoirPublisherOptions, RenoirReceivedMessage, RenoirRegionConfig,
    RenoirRegionHandle, RenoirRegionStats, RenoirSequenceNumber, RenoirSubscriberHandle,
    RenoirSubscriberOptions, RenoirTopicId, RenoirTopicInfo, RenoirTopicManagerHandle,
    RenoirTopicOptions, RenoirTopicStats,
};

pub use utils::{renoir_free_string, HANDLE_REGISTRY};

// Memory management API
pub use memory::{
    renoir_free_region_names, renoir_manager_clear_all, renoir_manager_create,
    renoir_manager_create_with_control, renoir_manager_destroy, renoir_manager_flush_all,
    renoir_manager_has_region, renoir_manager_list_regions, renoir_manager_region_count,
    renoir_manager_total_memory_usage, renoir_region_close, renoir_region_create,
    renoir_region_create_bump_allocator, renoir_region_destroy, renoir_region_get,
    renoir_region_metadata, renoir_region_name, renoir_region_open, renoir_region_size,
    renoir_region_stats,
};

// Buffer management API
pub use buffers::{
    renoir_buffer_get, renoir_buffer_info, renoir_buffer_pool_create, renoir_buffer_pool_destroy,
    renoir_buffer_pool_stats, renoir_buffer_return,
};

// Topic management API
pub use topics::{
    renoir_message_release, renoir_publish, renoir_publish_commit, renoir_publish_reserve,
    renoir_publish_try, renoir_publisher_create, renoir_publisher_destroy,
    renoir_publisher_has_subscribers, renoir_publisher_stats, renoir_publisher_topic_name,
    renoir_subscribe_drain, renoir_subscribe_peek, renoir_subscribe_read_next,
    renoir_subscriber_create, renoir_subscriber_destroy, renoir_subscriber_has_messages,
    renoir_subscriber_pending_count, renoir_subscriber_stats, renoir_subscriber_topic_name,
    renoir_topic_count, renoir_topic_list, renoir_topic_list_free, renoir_topic_lookup,
    renoir_topic_manager_create, renoir_topic_manager_destroy, renoir_topic_register,
    renoir_topic_remove, renoir_topic_stats,
};

#[cfg(target_os = "linux")]
pub use topics::{renoir_subscriber_get_event_fd, renoir_wait_event};

// Version API
pub use version::{
    renoir_version_major, renoir_version_minor, renoir_version_patch, renoir_version_string,
};
