//! Message types and payload handling

use std::sync::atomic::{AtomicU32, Ordering};
use crate::error::{RenoirError, Result};
use super::header::MessageHeader;

/// Large message descriptor for shared buffer pool
#[derive(Debug)]
#[repr(C)]
pub struct MessageDescriptor {
    /// Buffer pool ID
    pub pool_id: u32,
    /// Buffer handle within pool
    pub buffer_handle: u32,
    /// Actual payload size
    pub payload_size: u32,
    /// Reference count for garbage collection
    pub ref_count: AtomicU32,
}

impl MessageDescriptor {
    /// Create a new message descriptor
    pub fn new(pool_id: u32, buffer_handle: u32, payload_size: u32) -> Self {
        Self {
            pool_id,
            buffer_handle,
            payload_size,
            ref_count: AtomicU32::new(1),
        }
    }
    
    /// Increment reference count
    pub fn add_ref(&self) -> u32 {
        self.ref_count.fetch_add(1, Ordering::SeqCst)
    }
    
    /// Decrement reference count, returns true if should be freed
    pub fn release(&self) -> bool {
        self.ref_count.fetch_sub(1, Ordering::SeqCst) == 1
    }
}

/// Message content - either inline or descriptor-based
#[derive(Debug)]
pub enum MessagePayload {
    /// Small message stored inline in ring buffer
    Inline(Vec<u8>),
    /// Large message stored in shared buffer pool
    Descriptor(MessageDescriptor),
}

/// Complete message with header and payload
#[derive(Debug)]
pub struct Message {
    /// Message header with metadata
    pub header: MessageHeader,
    /// Message payload (inline or descriptor)
    pub payload: MessagePayload,
}

impl Message {
    /// Create a new inline message
    pub fn new_inline(topic_id: u32, sequence: u64, payload: Vec<u8>) -> Self {
        let mut header = MessageHeader::new(topic_id, sequence, payload.len() as u32);
        header.set_checksum(&payload);
        
        Self {
            header,
            payload: MessagePayload::Inline(payload),
        }
    }
    
    /// Create a new descriptor-based message
    pub fn new_descriptor(topic_id: u32, sequence: u64, descriptor: MessageDescriptor) -> Self {
        // For descriptor messages, the payload_length is the size of the descriptor itself,
        // not the size of the data it references
        let header = MessageHeader::new(topic_id, sequence, std::mem::size_of::<MessageDescriptor>() as u32);
        
        Self {
            header,
            payload: MessagePayload::Descriptor(descriptor),
        }
    }
    
    /// Get the total message size (header + payload)
    pub fn total_size(&self) -> usize {
        MessageHeader::SIZE + match &self.payload {
            MessagePayload::Inline(data) => data.len(),
            MessagePayload::Descriptor(_) => std::mem::size_of::<MessageDescriptor>(),
        }
    }
    
    /// Validate the message integrity
    pub fn validate(&self) -> Result<()> {
        self.header.validate()?;
        
        match &self.payload {
            MessagePayload::Inline(data) => {
                if !self.header.verify_checksum(data) {
                    return Err(RenoirError::invalid_parameter(
                        "checksum",
                        "Message checksum validation failed"
                    ));
                }
            }
            MessagePayload::Descriptor(_) => {
                // Descriptor validation would require access to buffer pool
            }
        }
        
        Ok(())
    }
}