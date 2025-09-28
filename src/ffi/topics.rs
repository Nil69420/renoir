//! FFI functions for topic management

use std::{
    ffi::c_char,
    ptr::null_mut,
    sync::Arc,
};

use crate::{
    topic_manager_modules::TopicManager,
    topic::{TopicConfig, TopicPattern, TopicQoS, MessagePayload},
};

use super::{
    types::{
        RenoirErrorCode, RenoirTopicManagerHandle, RenoirPublisherHandle, 
        RenoirSubscriberHandle
    },
    utils::{HANDLE_REGISTRY, c_str_to_string},
};

/// Create a new topic manager
#[no_mangle]
pub extern "C" fn renoir_topic_manager_create() -> RenoirTopicManagerHandle {
    match TopicManager::new() {
        Ok(manager) => {
            let manager = Arc::new(manager);
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let id = registry.next_id;
            registry.next_id += 1;
            registry.topic_managers.insert(id, manager);
            id as RenoirTopicManagerHandle
        }
        Err(_) => null_mut(),
    }
}

/// Destroy a topic manager
#[no_mangle]
pub extern "C" fn renoir_topic_manager_destroy(handle: RenoirTopicManagerHandle) -> RenoirErrorCode {
    if handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let id = handle as usize;
    
    if registry.topic_managers.remove(&id).is_some() {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Create a topic
/// 
/// # Parameters
/// - `manager_handle`: Handle to the topic manager
/// - `name`: Topic name (null-terminated C string)
/// - `message_size`: Maximum message size in bytes
/// - `ring_size`: Size of the ring buffer (must be power of 2)
/// - `is_spsc`: Whether to use Single Producer Single Consumer (true) or Multi Producer Multi Consumer (false)
/// 
/// # Returns
/// Topic ID (> 0) on success, 0 on failure
#[no_mangle]
pub extern "C" fn renoir_topic_create(
    manager_handle: RenoirTopicManagerHandle,
    name: *const c_char,
    message_size: u32,
    ring_size: u32,
    is_spsc: bool,
) -> u32 {
    if manager_handle.is_null() || name.is_null() {
        return 0;
    }

    let topic_name = match c_str_to_string(name) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let pattern = if is_spsc {
        TopicPattern::SPSC
    } else {
        TopicPattern::MPMC
    };

    let config = TopicConfig {
        name: topic_name,
        pattern,
        ring_capacity: ring_size as usize,
        max_payload_size: message_size as usize,
        use_shared_pool: false,
        shared_pool_threshold: message_size as usize,
        enable_notifications: false,
        qos: TopicQoS::default(),
    };

    let registry = HANDLE_REGISTRY.lock().unwrap();
    let id = manager_handle as usize;
    
    if let Some(manager) = registry.topic_managers.get(&id) {
        match manager.create_topic(config) {
            Ok(topic_id) => topic_id,
            Err(_) => 0,
        }
    } else {
        0
    }
}

/// Create a publisher for a topic
#[no_mangle]
pub extern "C" fn renoir_publisher_create(
    manager_handle: RenoirTopicManagerHandle,
    topic_id: u32,
) -> RenoirPublisherHandle {
    if manager_handle.is_null() || topic_id == 0 {
        return null_mut();
    }

    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let manager_id = manager_handle as usize;
    
    if let Some(manager) = registry.topic_managers.get(&manager_id) {
        let topic_name = match manager.get_topic_name(topic_id) {
            Some(name) => name,
            None => return null_mut(),
        };
        match manager.create_publisher(&topic_name) {
            Ok(publisher) => {
                let id = registry.next_id;
                registry.next_id += 1;
                registry.publishers.insert(id, publisher);
                id as RenoirPublisherHandle
            }
            Err(_) => null_mut(),
        }
    } else {
        null_mut()
    }
}

/// Create a subscriber for a topic
#[no_mangle]
pub extern "C" fn renoir_subscriber_create(
    manager_handle: RenoirTopicManagerHandle,
    topic_id: u32,
) -> RenoirSubscriberHandle {
    if manager_handle.is_null() || topic_id == 0 {
        return null_mut();
    }

    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let manager_id = manager_handle as usize;
    
    if let Some(manager) = registry.topic_managers.get(&manager_id) {
        let topic_name = match manager.get_topic_name(topic_id) {
            Some(name) => name,
            None => return null_mut(),
        };
        match manager.create_subscriber(&topic_name) {
            Ok(subscriber) => {
                let id = registry.next_id;
                registry.next_id += 1;
                registry.subscribers.insert(id, subscriber);
                id as RenoirSubscriberHandle
            }
            Err(_) => null_mut(),
        }
    } else {
        null_mut()
    }
}

/// Publish a message to a topic
/// 
/// # Parameters
/// - `publisher_handle`: Handle to the publisher
/// - `data`: Pointer to message data
/// - `size`: Size of the message data
/// 
/// # Returns
/// Success/error code
#[no_mangle]
pub extern "C" fn renoir_publish_message(
    publisher_handle: RenoirPublisherHandle,
    data: *const u8,
    size: u32,
) -> RenoirErrorCode {
    if publisher_handle.is_null() || data.is_null() || size == 0 {
        return RenoirErrorCode::InvalidParameter;
    }

    let registry = HANDLE_REGISTRY.lock().unwrap();
    let id = publisher_handle as usize;
    
    if let Some(publisher) = registry.publishers.get(&id) {
        let payload_data = unsafe { 
            std::slice::from_raw_parts(data, size as usize) 
        };
        
        match publisher.publish(payload_data.to_vec()) {
            Ok(_) => RenoirErrorCode::Success,
            Err(err) => err.into(),
        }
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Receive a message from a topic (non-blocking)
/// 
/// # Parameters
/// - `subscriber_handle`: Handle to the subscriber
/// - `data`: Buffer to store message data
/// - `buffer_size`: Size of the buffer
/// - `actual_size`: Pointer to store actual message size
/// 
/// # Returns
/// Success/error code
#[no_mangle]
pub extern "C" fn renoir_receive_message(
    subscriber_handle: RenoirSubscriberHandle,
    data: *mut u8,
    buffer_size: u32,
    actual_size: *mut u32,
) -> RenoirErrorCode {
    if subscriber_handle.is_null() || data.is_null() || actual_size.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let registry = HANDLE_REGISTRY.lock().unwrap();
    let id = subscriber_handle as usize;
    
    if let Some(subscriber) = registry.subscribers.get(&id) {
        match subscriber.subscribe() {
            Ok(Some(message)) => {
                let payload_data = match &message.payload {
                    MessagePayload::Inline(data) => data,
                    MessagePayload::Descriptor(_) => {
                        // For simplicity, we don't handle descriptor messages in C API yet
                        return RenoirErrorCode::SerializationError;
                    }
                };
                
                let msg_size = payload_data.len() as u32;
                unsafe { *actual_size = msg_size; }
                
                if msg_size > buffer_size {
                    return RenoirErrorCode::InsufficientSpace;
                }
                
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        payload_data.as_ptr(),
                        data,
                        msg_size as usize,
                    );
                }
                
                RenoirErrorCode::Success
            }
            Ok(None) => RenoirErrorCode::BufferEmpty,
            Err(err) => err.into(),
        }
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Destroy a publisher
#[no_mangle]
pub extern "C" fn renoir_publisher_destroy(handle: RenoirPublisherHandle) -> RenoirErrorCode {
    if handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let id = handle as usize;
    
    if registry.publishers.remove(&id).is_some() {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Destroy a subscriber
#[no_mangle]
pub extern "C" fn renoir_subscriber_destroy(handle: RenoirSubscriberHandle) -> RenoirErrorCode {
    if handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let id = handle as usize;
    
    if registry.subscribers.remove(&id).is_some() {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Remove a topic
#[no_mangle]
pub extern "C" fn renoir_topic_remove(
    manager_handle: RenoirTopicManagerHandle,
    topic_id: u32,
) -> RenoirErrorCode {
    if manager_handle.is_null() || topic_id == 0 {
        return RenoirErrorCode::InvalidParameter;
    }

    let registry = HANDLE_REGISTRY.lock().unwrap();
    let id = manager_handle as usize;
    
    if let Some(manager) = registry.topic_managers.get(&id) {
        let topic_name = match manager.get_topic_name(topic_id) {
            Some(name) => name,
            None => return RenoirErrorCode::InvalidParameter,
        };
        match manager.remove_topic(&topic_name) {
            Ok(_) => RenoirErrorCode::Success,
            Err(err) => err.into(),
        }
    } else {
        RenoirErrorCode::InvalidParameter
    }
}