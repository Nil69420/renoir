//! FFI functions for topic-based messaging system
//!
//! This module provides a complete C API for Renoir's topic-based pub/sub system,
//! designed for integration with ROS2 and other robotics frameworks.
//!
//! # Architecture
//! - TopicManager: Central registry for topics
//! - Publisher: Sends messages to topics
//! - Subscriber: Receives messages from topics
//! - Message: Payload container with metadata
//!
//! # Thread Safety
//! All handles are thread-safe and can be shared across threads.

use std::ffi::c_char;
use std::sync::Arc;
use std::time::Duration;

use crate::{
    topic::{MessagePayload, TopicConfig, TopicPattern, TopicQoS, Reliability, config::TopicMessageFormat},
    topic_manager_modules::TopicManager,
};

use super::{
    types::*,
    utils::{c_str_to_string, string_to_c_str, HANDLE_REGISTRY},
};

// ============================================================================
// Topic Manager API
// ============================================================================

/// Create a new topic manager.
///
/// The topic manager is the central coordinator for all topic-based communication.
/// It maintains a registry of topics and manages publisher/subscriber instances.
///
/// # Returns
/// - Valid handle on success
/// - Null pointer on failure
///
/// # Thread Safety
/// This function is thread-safe.
///
/// # Example
/// ```c
/// RenoirTopicManagerHandle manager = renoir_topic_manager_create();
/// if (manager == NULL) {
///     // Handle error
/// }
/// ```
#[no_mangle]
pub extern "C" fn renoir_topic_manager_create() -> RenoirTopicManagerHandle {
    match TopicManager::new() {
        Ok(manager) => {
            let buffer_registry = manager.buffer_registry().clone();
            let manager = Arc::new(manager);
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let id = registry.store_topic_manager(manager, buffer_registry);
            id as RenoirTopicManagerHandle
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Destroy a topic manager
#[no_mangle]
pub extern "C" fn renoir_topic_manager_destroy(
    manager: RenoirTopicManagerHandle,
) -> RenoirErrorCode {
    if manager.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();

    if registry.remove_topic_manager(manager_id) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

// ============================================================================
// Topic Registration API
// ============================================================================

/// Register a new topic with the manager
#[no_mangle]
pub extern "C" fn renoir_topic_register(
    manager: RenoirTopicManagerHandle,
    topic_name: *const c_char,
    options: *const RenoirTopicOptions,
    topic_id: *mut RenoirTopicId,
) -> RenoirErrorCode {
    if manager.is_null() || topic_name.is_null() || options.is_null() || topic_id.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let manager = match registry.get_topic_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let name = match c_str_to_string(topic_name) {
        Ok(n) => n,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    let options = unsafe { &*options };

    // Convert options to TopicConfig
    let pattern = match options.pattern {
        0 => TopicPattern::SPSC,
        1 => TopicPattern::SPMC,
        2 => TopicPattern::MPSC,
        3 => TopicPattern::MPMC,
        _ => return RenoirErrorCode::InvalidParameter,
    };

    let config = TopicConfig {
        name: name.clone(),
        pattern,
        ring_capacity: options.ring_capacity,
        max_payload_size: options.max_payload_size,
        use_shared_pool: options.use_shared_pool,
        shared_pool_threshold: options.shared_pool_threshold,
        enable_notifications: options.enable_notifications,
        qos: TopicQoS {
            reliability: Reliability::BestEffort,
            ..Default::default()
        },
        message_format: TopicMessageFormat::default(),
    };

    drop(registry); // Release lock before calling manager

    match manager.create_topic(config) {
        Ok(id) => {
            unsafe {
                *topic_id = id as RenoirTopicId;
            }
            RenoirErrorCode::Success
        }
        Err(e) => {
            eprintln!("Failed to create topic '{}': {:?}", name, e);
            e.into()
        }
    }
}

/// Lookup an existing topic by name
#[no_mangle]
pub extern "C" fn renoir_topic_lookup(
    manager: RenoirTopicManagerHandle,
    topic_name: *const c_char,
    topic_id: *mut RenoirTopicId,
) -> RenoirErrorCode {
    if manager.is_null() || topic_name.is_null() || topic_id.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let manager = match registry.get_topic_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let name = match c_str_to_string(topic_name) {
        Ok(n) => n,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    drop(registry);

    if manager.has_topic(&name) {
        // Get topic ID from manager
        if let Some(id) = manager.get_topic_id(&name) {
            unsafe {
                *topic_id = id as RenoirTopicId;
            }
            return RenoirErrorCode::Success;
        }
    }
    
    RenoirErrorCode::RegionNotFound
}

/// Remove a topic from the manager
#[no_mangle]
pub extern "C" fn renoir_topic_remove(
    manager: RenoirTopicManagerHandle,
    topic_name: *const c_char,
) -> RenoirErrorCode {
    if manager.is_null() || topic_name.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let manager = match registry.get_topic_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let name = match c_str_to_string(topic_name) {
        Ok(n) => n,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    drop(registry);

    match manager.remove_topic(&name) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

/// Get the count of registered topics
#[no_mangle]
pub extern "C" fn renoir_topic_count(manager: RenoirTopicManagerHandle) -> usize {
    if manager.is_null() {
        return 0;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let manager = match registry.get_topic_manager(manager_id) {
        Some(m) => m,
        None => return 0,
    };

    manager.topic_count()
}

/// List all registered topics
#[no_mangle]
pub extern "C" fn renoir_topic_list(
    manager: RenoirTopicManagerHandle,
    topics: *mut *mut RenoirTopicInfo,
    count: *mut usize,
) -> RenoirErrorCode {
    if manager.is_null() || topics.is_null() || count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let manager = match registry.get_topic_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    // Get topic list from manager
    let topic_list = manager.list_topics();
    let topic_count = topic_list.len();

    if topic_count == 0 {
        unsafe {
            *topics = std::ptr::null_mut();
            *count = 0;
        }
        return RenoirErrorCode::Success;
    }

    // Allocate array for topic info
    let layout = std::alloc::Layout::array::<RenoirTopicInfo>(topic_count).unwrap();
    let ptr = unsafe { std::alloc::alloc(layout) as *mut RenoirTopicInfo };

    if ptr.is_null() {
        return RenoirErrorCode::OutOfMemory;
    }

    // Fill in topic information
    for (i, (name, topic_id, config)) in topic_list.iter().enumerate() {
        let topic_info = unsafe { ptr.add(i) };
        let name_cstr = string_to_c_str(name.clone());
        
        unsafe {
            (*topic_info).topic_id = *topic_id as RenoirTopicId;
            (*topic_info).name = name_cstr;
            (*topic_info).pattern = match config.pattern {
                crate::topic::TopicPattern::SPSC => 0,
                crate::topic::TopicPattern::SPMC => 1,
                crate::topic::TopicPattern::MPSC => 2,
                crate::topic::TopicPattern::MPMC => 3,
            };
            
            // Get stats to populate counts
            if let Ok(stats) = manager.topic_stats(name) {
                (*topic_info).publisher_count = stats.active_publishers.load(std::sync::atomic::Ordering::Relaxed) as usize;
                (*topic_info).subscriber_count = stats.active_subscribers.load(std::sync::atomic::Ordering::Relaxed) as usize;
            } else {
                (*topic_info).publisher_count = 0;
                (*topic_info).subscriber_count = 0;
            }
        }
    }

    unsafe {
        *topics = ptr;
        *count = topic_count;
    }

    RenoirErrorCode::Success
}

/// Free topic list returned by renoir_topic_list
#[no_mangle]
pub extern "C" fn renoir_topic_list_free(topics: *mut RenoirTopicInfo, count: usize) {
    if !topics.is_null() && count > 0 {
        unsafe {
            let _ = Vec::from_raw_parts(topics, count, count);
        }
    }
}

// ============================================================================
// Publisher API
// ============================================================================

/// Create a publisher for a topic
#[no_mangle]
pub extern "C" fn renoir_publisher_create(
    manager: RenoirTopicManagerHandle,
    topic_name: *const c_char,
    _options: *const RenoirPublisherOptions,
    publisher: *mut RenoirPublisherHandle,
) -> RenoirErrorCode {
    if manager.is_null() || topic_name.is_null() || publisher.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let manager = match registry.get_topic_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let name = match c_str_to_string(topic_name) {
        Ok(n) => n,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    drop(registry);

    match manager.create_publisher(&name) {
        Ok(pub_handle) => {
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let id = registry.store_publisher(Arc::new(pub_handle), manager_id);
            unsafe {
                *publisher = id as RenoirPublisherHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Destroy a publisher
#[no_mangle]
pub extern "C" fn renoir_publisher_destroy(publisher: RenoirPublisherHandle) -> RenoirErrorCode {
    if publisher.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pub_id = publisher as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();

    if registry.remove_publisher(pub_id) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Publish a message to a topic
#[no_mangle]
pub extern "C" fn renoir_publish(
    publisher: RenoirPublisherHandle,
    payload_ptr: *const u8,
    payload_len: usize,
    _metadata: *const RenoirMessageMetadata,
    sequence: *mut RenoirSequenceNumber,
) -> RenoirErrorCode {
    if publisher.is_null() || payload_ptr.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pub_id = publisher as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let pub_handle = match registry.get_publisher(pub_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    // Copy payload to Vec
    let payload = unsafe { std::slice::from_raw_parts(payload_ptr, payload_len) }.to_vec();

    drop(registry);

    match pub_handle.publish(payload) {
        Ok(()) => {
            // Get sequence number from stats if requested
            if !sequence.is_null() {
                let seq = pub_handle.stats().current_sequence.load(std::sync::atomic::Ordering::Relaxed);
                unsafe {
                    *sequence = seq;
                }
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Try to publish (non-blocking)
#[no_mangle]
pub extern "C" fn renoir_publish_try(
    publisher: RenoirPublisherHandle,
    payload_ptr: *const u8,
    payload_len: usize,
    _metadata: *const RenoirMessageMetadata,
    sequence: *mut RenoirSequenceNumber,
) -> RenoirErrorCode {
    if publisher.is_null() || payload_ptr.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pub_id = publisher as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let pub_handle = match registry.get_publisher(pub_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let payload = unsafe { std::slice::from_raw_parts(payload_ptr, payload_len) }.to_vec();

    drop(registry);

    match pub_handle.try_publish(payload) {
        Ok(published) => {
            if published {
                if !sequence.is_null() {
                    let seq = pub_handle.stats().current_sequence.load(std::sync::atomic::Ordering::Relaxed);
                    unsafe {
                        *sequence = seq;
                    }
                }
                RenoirErrorCode::Success
            } else {
                RenoirErrorCode::BufferFull
            }
        }
        Err(e) => e.into(),
    }
}

/// Reserve space for zero-copy publishing
#[no_mangle]
pub extern "C" fn renoir_publish_reserve(
    publisher: RenoirPublisherHandle,
    size: usize,
    payload_ptr: *mut *mut u8,
    reserved: *mut RenoirReservedBufferHandle,
) -> RenoirErrorCode {
    if publisher.is_null() || payload_ptr.is_null() || reserved.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    if size == 0 {
        return RenoirErrorCode::InvalidParameter;
    }

    // Allocate a buffer for zero-copy publishing
    let mut buffer = vec![0u8; size];
    let buffer_ptr = buffer.as_mut_ptr();

    // Store the buffer in the registry
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer_id = registry.next_id;
    registry.next_id += 1;
    registry.reserved_buffers.insert(buffer_id, buffer);

    unsafe {
        *payload_ptr = buffer_ptr;
        *reserved = buffer_id as RenoirReservedBufferHandle;
    }

    RenoirErrorCode::Success
}

/// Commit a reserved buffer for publishing
#[no_mangle]
pub extern "C" fn renoir_publish_commit(
    publisher: RenoirPublisherHandle,
    reserved: RenoirReservedBufferHandle,
    actual_len: usize,
    _metadata: *const RenoirMessageMetadata,
    sequence: *mut RenoirSequenceNumber,
) -> RenoirErrorCode {
    if publisher.is_null() || reserved.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pub_id = publisher as usize;
    let buffer_id = reserved as usize;

    // Get the reserved buffer
    let buffer = {
        let mut registry = HANDLE_REGISTRY.lock().unwrap();
        match registry.reserved_buffers.remove(&buffer_id) {
            Some(buf) => buf,
            None => return RenoirErrorCode::InvalidParameter,
        }
    };

    // Validate actual_len doesn't exceed reserved size
    if actual_len > buffer.len() {
        return RenoirErrorCode::InvalidParameter;
    }

    // Truncate buffer to actual length
    let payload = buffer[..actual_len].to_vec();

    // Get the publisher handle
    let pub_handle = {
        let registry = HANDLE_REGISTRY.lock().unwrap();
        match registry.get_publisher(pub_id) {
            Some(p) => p,
            None => return RenoirErrorCode::InvalidParameter,
        }
    };

    // Publish the message (sequence is generated internally)
    // Note: metadata parameter is currently unused as Publisher::publish
    // doesn't support custom metadata yet. This could be extended in the future.
    match pub_handle.publish(payload) {
        Ok(()) => {
            // Get the sequence number from publisher stats
            if !sequence.is_null() {
                let stats = pub_handle.stats();
                unsafe {
                    *sequence = stats.messages_published.load(std::sync::atomic::Ordering::Relaxed);
                }
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

// ============================================================================
// Subscriber API
// ============================================================================

/// Create a subscriber for a topic
#[no_mangle]
pub extern "C" fn renoir_subscriber_create(
    manager: RenoirTopicManagerHandle,
    topic_name: *const c_char,
    _options: *const RenoirSubscriberOptions,
    subscriber: *mut RenoirSubscriberHandle,
) -> RenoirErrorCode {
    if manager.is_null() || topic_name.is_null() || subscriber.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let manager = match registry.get_topic_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let name = match c_str_to_string(topic_name) {
        Ok(n) => n,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    drop(registry);

    match manager.create_subscriber(&name) {
        Ok(sub_handle) => {
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let id = registry.store_subscriber(Arc::new(sub_handle), manager_id);
            unsafe {
                *subscriber = id as RenoirSubscriberHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Destroy a subscriber
#[no_mangle]
pub extern "C" fn renoir_subscriber_destroy(
    subscriber: RenoirSubscriberHandle,
) -> RenoirErrorCode {
    if subscriber.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let sub_id = subscriber as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();

    if registry.remove_subscriber(sub_id) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Read the next message (blocking or non-blocking based on options)
#[no_mangle]
pub extern "C" fn renoir_subscribe_read_next(
    subscriber: RenoirSubscriberHandle,
    message: *mut RenoirReceivedMessage,
    timeout_ms: u64,
) -> RenoirErrorCode {
    if subscriber.is_null() || message.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let sub_id = subscriber as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let sub_handle = match registry.get_subscriber(sub_id) {
        Some(s) => s,
        None => return RenoirErrorCode::InvalidParameter,
    };

    drop(registry);

    // Try to receive message with timeout
    let received_msg = if timeout_ms > 0 {
        #[cfg(target_os = "linux")]
        {
            let timeout = Duration::from_millis(timeout_ms);
            match sub_handle.wait_for_message(Some(timeout)) {
                Ok(Some(msg)) => Some(msg),
                Ok(None) => None,
                Err(e) => return e.into(),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Fallback: polling with timeout
            let start = std::time::Instant::now();
            loop {
                if let Ok(Some(msg)) = sub_handle.try_subscribe() {
                    break Some(msg);
                }
                if start.elapsed() >= Duration::from_millis(timeout_ms) {
                    break None;
                }
                std::thread::sleep(Duration::from_micros(100));
            }
        }
    } else {
        match sub_handle.try_subscribe() {
            Ok(msg) => msg,
            Err(e) => return e.into(),
        }
    };

    match received_msg {
        Some(msg) => {
            // Get buffer registry for this subscriber
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let manager_id = registry.get_subscriber_manager_id(sub_id);
            let buffer_registry = manager_id.and_then(|id| registry.get_buffer_registry(id));
            
            let mut descriptor_buffer_id: Option<usize> = None;

            unsafe {
                // Fill in payload pointer and length based on message type
                match &msg.payload {
                    MessagePayload::Inline(data) => {
                        (*message).payload_ptr = data.as_ptr();
                        (*message).payload_len = data.len();
                    }
                    MessagePayload::Descriptor(desc) => {
                        // Resolve descriptor to actual buffer data
                        if let Some(buf_registry) = buffer_registry {
                            match buf_registry.get_buffer_data(desc) {
                                Ok(buffer) => {
                                    // Store buffer to keep it alive
                                    let buf_id = registry.next_id;
                                    registry.next_id += 1;
                                    registry.descriptor_buffers.insert(buf_id, buffer.clone());
                                    descriptor_buffer_id = Some(buf_id);
                                    
                                    // Return pointer and size
                                    (*message).payload_ptr = buffer.as_ptr();
                                    (*message).payload_len = desc.payload_size as usize;
                                }
                                Err(_) => {
                                    // Failed to resolve descriptor
                                    (*message).payload_ptr = std::ptr::null();
                                    (*message).payload_len = 0;
                                }
                            }
                        } else {
                            // No buffer registry available
                            (*message).payload_ptr = std::ptr::null();
                            (*message).payload_len = 0;
                        }
                    }
                }

                // Fill metadata
                (*message).metadata.timestamp_ns = msg.header.timestamp;
                (*message).metadata.sequence_number = msg.header.sequence;
                (*message).metadata.source_id = msg.header.topic_id;
                (*message).metadata.message_type = 0;
                (*message).metadata.flags = 0; // No flags field in MessageHeader
            }

            // Store message in registry and return handle
            let msg_id = registry.store_message(Box::new(msg));
            
            // Associate descriptor buffer with message for cleanup
            if let Some(buf_id) = descriptor_buffer_id {
                registry.associate_descriptor_buffer(msg_id, buf_id);
            }

            unsafe {
                (*message).handle = msg_id as RenoirReceivedMessageHandle;
            }

            drop(registry);

            RenoirErrorCode::Success
        }
        None => RenoirErrorCode::BufferEmpty,
    }
}

/// Peek at the next message without consuming it
#[no_mangle]
pub extern "C" fn renoir_subscribe_peek(
    subscriber: RenoirSubscriberHandle,
    message: *mut RenoirReceivedMessage,
) -> RenoirErrorCode {
    if subscriber.is_null() || message.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let sub_id = subscriber as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let sub_handle = match registry.get_subscriber(sub_id) {
        Some(s) => s,
        None => return RenoirErrorCode::InvalidParameter,
    };

    drop(registry);

    // Try to peek at the next message
    let peeked_msg = match sub_handle.peek() {
        Ok(msg) => msg,
        Err(e) => return e.into(),
    };

    match peeked_msg {
        Some(msg) => {
            // Get buffer registry for this subscriber
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let manager_id = registry.get_subscriber_manager_id(sub_id);
            let buffer_registry = manager_id.and_then(|id| registry.get_buffer_registry(id));
            
            let mut descriptor_buffer_id: Option<usize> = None;

            unsafe {
                // Fill in payload pointer and length based on message type
                match &msg.payload {
                    MessagePayload::Inline(data) => {
                        (*message).payload_ptr = data.as_ptr();
                        (*message).payload_len = data.len();
                    }
                    MessagePayload::Descriptor(desc) => {
                        // Resolve descriptor to actual buffer data
                        if let Some(buf_registry) = buffer_registry {
                            match buf_registry.get_buffer_data(desc) {
                                Ok(buffer) => {
                                    // Store buffer to keep it alive
                                    let buf_id = registry.next_id;
                                    registry.next_id += 1;
                                    registry.descriptor_buffers.insert(buf_id, buffer.clone());
                                    descriptor_buffer_id = Some(buf_id);
                                    
                                    // Return pointer and size
                                    (*message).payload_ptr = buffer.as_ptr();
                                    (*message).payload_len = desc.payload_size as usize;
                                }
                                Err(_) => {
                                    // Failed to resolve descriptor
                                    (*message).payload_ptr = std::ptr::null();
                                    (*message).payload_len = 0;
                                }
                            }
                        } else {
                            // No buffer registry available
                            (*message).payload_ptr = std::ptr::null();
                            (*message).payload_len = 0;
                        }
                    }
                }

                // Fill metadata
                (*message).metadata.timestamp_ns = msg.header.timestamp;
                (*message).metadata.sequence_number = msg.header.sequence;
                (*message).metadata.source_id = msg.header.topic_id;
                (*message).metadata.message_type = 0;
                (*message).metadata.flags = 0;
            }

            // Store message in registry and return handle
            let msg_id = registry.store_message(Box::new(msg));
            
            // Associate descriptor buffer with message for cleanup
            if let Some(buf_id) = descriptor_buffer_id {
                registry.associate_descriptor_buffer(msg_id, buf_id);
            }

            unsafe {
                (*message).handle = msg_id as RenoirReceivedMessageHandle;
            }

            drop(registry);

            RenoirErrorCode::Success
        }
        None => RenoirErrorCode::BufferEmpty,
    }
}

/// Release a received message
#[no_mangle]
pub extern "C" fn renoir_message_release(message_handle: RenoirReceivedMessageHandle) -> RenoirErrorCode {
    if message_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let msg_id = message_handle as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();

    if registry.remove_message(msg_id) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Drain all pending messages
#[no_mangle]
pub extern "C" fn renoir_subscribe_drain(
    subscriber: RenoirSubscriberHandle,
    drained_count: *mut usize,
) -> RenoirErrorCode {
    if subscriber.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let sub_id = subscriber as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let sub_handle = match registry.get_subscriber(sub_id) {
        Some(s) => s,
        None => return RenoirErrorCode::InvalidParameter,
    };

    drop(registry);

    match sub_handle.drain_messages() {
        Ok(messages) => {
            if !drained_count.is_null() {
                unsafe {
                    *drained_count = messages.len();
                }
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

// ============================================================================
// Notification API
// ============================================================================

/// Get the event file descriptor for a subscriber (Linux only)
#[cfg(target_os = "linux")]
#[no_mangle]
pub extern "C" fn renoir_subscriber_get_event_fd(
    subscriber: RenoirSubscriberHandle,
    fd: *mut std::os::fd::RawFd,
) -> RenoirErrorCode {
    if subscriber.is_null() || fd.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let sub_id = subscriber as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let sub_handle = match registry.get_subscriber(sub_id) {
        Some(s) => s,
        None => return RenoirErrorCode::InvalidParameter,
    };

    match sub_handle.notification_fd() {
        Some(event_fd) => {
            unsafe {
                *fd = event_fd;
            }
            RenoirErrorCode::Success
        }
        None => RenoirErrorCode::PlatformError,
    }
}

/// Wait on an event fd with timeout (Linux only)
#[cfg(target_os = "linux")]
#[no_mangle]
pub extern "C" fn renoir_wait_event(fd: std::os::fd::RawFd, timeout_ms: u64) -> RenoirErrorCode {
    use nix::poll::{poll, PollFd, PollFlags};
    use std::os::fd::BorrowedFd;

    let timeout = if timeout_ms > 0 {
        timeout_ms as i32
    } else {
        -1 // Infinite wait
    };

    // SAFETY: The caller must ensure the fd is valid for the duration of the poll
    let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
    let mut poll_fds = [PollFd::new(&borrowed_fd, PollFlags::POLLIN)];

    match poll(&mut poll_fds, timeout) {
        Ok(n) if n > 0 => RenoirErrorCode::Success,
        Ok(_) => RenoirErrorCode::BufferEmpty, // Timeout
        Err(_) => RenoirErrorCode::IoError,
    }
}

// ============================================================================
// Statistics API
// ============================================================================

/// Get topic statistics
#[no_mangle]
pub extern "C" fn renoir_topic_stats(
    manager: RenoirTopicManagerHandle,
    topic_name: *const c_char,
    stats: *mut RenoirTopicStats,
) -> RenoirErrorCode {
    if manager.is_null() || topic_name.is_null() || stats.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let manager = match registry.get_topic_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let name = match c_str_to_string(topic_name) {
        Ok(n) => n,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    drop(registry);

    // Get topic ID
    let topic_id = manager.get_topic_id(&name).unwrap_or(0);

    match manager.topic_stats(&name) {
        Ok(topic_stats) => {
            unsafe {
                (*stats).topic_id = topic_id as RenoirTopicId;
                (*stats).publisher_count = topic_stats.active_publishers.load(std::sync::atomic::Ordering::Relaxed) as usize;
                (*stats).subscriber_count = topic_stats.active_subscribers.load(std::sync::atomic::Ordering::Relaxed) as usize;
                (*stats).messages_published = topic_stats.messages_published.load(std::sync::atomic::Ordering::Relaxed);
                (*stats).messages_consumed = topic_stats.messages_consumed.load(std::sync::atomic::Ordering::Relaxed);
                (*stats).messages_dropped = topic_stats.messages_dropped.load(std::sync::atomic::Ordering::Relaxed);
                (*stats).bytes_transferred = topic_stats.avg_message_size.load(std::sync::atomic::Ordering::Relaxed) * topic_stats.messages_published.load(std::sync::atomic::Ordering::Relaxed);
                // Latency tracking would require timestamp collection - not implemented yet
                (*stats).average_latency_ns = 0;
                (*stats).peak_latency_ns = 0;
                (*stats).throughput_mbps = 0.0;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Get publisher statistics
#[no_mangle]
pub extern "C" fn renoir_publisher_stats(
    publisher: RenoirPublisherHandle,
    stats: *mut RenoirTopicStats,
) -> RenoirErrorCode {
    if publisher.is_null() || stats.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pub_id = publisher as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let pub_handle = match registry.get_publisher(pub_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let topic_stats = pub_handle.stats();

    unsafe {
        (*stats).topic_id = 0;
        (*stats).publisher_count = topic_stats.active_publishers.load(std::sync::atomic::Ordering::Relaxed) as usize;
        (*stats).subscriber_count = topic_stats.active_subscribers.load(std::sync::atomic::Ordering::Relaxed) as usize;
        (*stats).messages_published = topic_stats.messages_published.load(std::sync::atomic::Ordering::Relaxed);
        (*stats).messages_consumed = topic_stats.messages_consumed.load(std::sync::atomic::Ordering::Relaxed);
        (*stats).messages_dropped = topic_stats.messages_dropped.load(std::sync::atomic::Ordering::Relaxed);
        (*stats).bytes_transferred = topic_stats.avg_message_size.load(std::sync::atomic::Ordering::Relaxed) * topic_stats.messages_published.load(std::sync::atomic::Ordering::Relaxed);
        (*stats).average_latency_ns = 0;
        (*stats).peak_latency_ns = 0;
        (*stats).throughput_mbps = 0.0;
    }

    RenoirErrorCode::Success
}

/// Get subscriber statistics
#[no_mangle]
pub extern "C" fn renoir_subscriber_stats(
    subscriber: RenoirSubscriberHandle,
    stats: *mut RenoirTopicStats,
) -> RenoirErrorCode {
    if subscriber.is_null() || stats.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let sub_id = subscriber as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let sub_handle = match registry.get_subscriber(sub_id) {
        Some(s) => s,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let topic_stats = sub_handle.stats();

    unsafe {
        (*stats).topic_id = 0;
        (*stats).publisher_count = topic_stats.active_publishers.load(std::sync::atomic::Ordering::Relaxed) as usize;
        (*stats).subscriber_count = topic_stats.active_subscribers.load(std::sync::atomic::Ordering::Relaxed) as usize;
        (*stats).messages_published = topic_stats.messages_published.load(std::sync::atomic::Ordering::Relaxed);
        (*stats).messages_consumed = topic_stats.messages_consumed.load(std::sync::atomic::Ordering::Relaxed);
        (*stats).messages_dropped = topic_stats.messages_dropped.load(std::sync::atomic::Ordering::Relaxed);
        (*stats).bytes_transferred = topic_stats.avg_message_size.load(std::sync::atomic::Ordering::Relaxed) * topic_stats.messages_published.load(std::sync::atomic::Ordering::Relaxed);
        (*stats).average_latency_ns = 0;
        (*stats).peak_latency_ns = 0;
        (*stats).throughput_mbps = 0.0;
    }

    RenoirErrorCode::Success
}

// ============================================================================
// Helper/Utility Functions
// ============================================================================

/// Check if publisher has active subscribers
#[no_mangle]
pub extern "C" fn renoir_publisher_has_subscribers(
    publisher: RenoirPublisherHandle,
    has_subscribers: *mut bool,
) -> RenoirErrorCode {
    if publisher.is_null() || has_subscribers.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pub_id = publisher as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let pub_handle = match registry.get_publisher(pub_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *has_subscribers = pub_handle.has_subscribers();
    }

    RenoirErrorCode::Success
}

/// Check if subscriber has pending messages
#[no_mangle]
pub extern "C" fn renoir_subscriber_has_messages(
    subscriber: RenoirSubscriberHandle,
    has_messages: *mut bool,
) -> RenoirErrorCode {
    if subscriber.is_null() || has_messages.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let sub_id = subscriber as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let sub_handle = match registry.get_subscriber(sub_id) {
        Some(s) => s,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *has_messages = sub_handle.has_new_messages();
    }

    RenoirErrorCode::Success
}

/// Get pending message count for subscriber
#[no_mangle]
pub extern "C" fn renoir_subscriber_pending_count(
    subscriber: RenoirSubscriberHandle,
    count: *mut usize,
) -> RenoirErrorCode {
    if subscriber.is_null() || count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let sub_id = subscriber as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let sub_handle = match registry.get_subscriber(sub_id) {
        Some(s) => s,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *count = sub_handle.pending_messages();
    }

    RenoirErrorCode::Success
}

/// Get topic name from publisher
#[no_mangle]
pub extern "C" fn renoir_publisher_topic_name(
    publisher: RenoirPublisherHandle,
) -> *const c_char {
    if publisher.is_null() {
        return std::ptr::null();
    }

    let pub_id = publisher as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let pub_handle = match registry.get_publisher(pub_id) {
        Some(p) => p,
        None => return std::ptr::null(),
    };

    let name = pub_handle.topic_name();
    string_to_c_str(name.to_string())
}

/// Get topic name from subscriber
#[no_mangle]
pub extern "C" fn renoir_subscriber_topic_name(
    subscriber: RenoirSubscriberHandle,
) -> *const c_char {
    if subscriber.is_null() {
        return std::ptr::null();
    }

    let sub_id = subscriber as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let sub_handle = match registry.get_subscriber(sub_id) {
        Some(s) => s,
        None => return std::ptr::null(),
    };

    let name = sub_handle.topic_name();
    string_to_c_str(name.to_string())
}
