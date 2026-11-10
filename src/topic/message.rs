//! Message types and payload handling

use super::header::MessageHeader;
use crate::topic::header::{FORMAT_TYPE_DESCRIPTOR, FORMAT_TYPE_INLINE};
use std::sync::atomic::{AtomicU32, Ordering};

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
    /// Identity of the pool manager instance that owns the buffer; zero when unset
    pub owner_id: u64,
}

impl Clone for MessageDescriptor {
    fn clone(&self) -> Self {
        Self {
            pool_id: self.pool_id,
            buffer_handle: self.buffer_handle,
            payload_size: self.payload_size,
            ref_count: AtomicU32::new(self.ref_count.load(Ordering::Relaxed)),
            owner_id: self.owner_id,
        }
    }
}

impl MessageDescriptor {
    /// Create a new message descriptor
    pub fn new(pool_id: u32, buffer_handle: u32, payload_size: u32) -> Self {
        Self {
            pool_id,
            buffer_handle,
            payload_size,
            ref_count: AtomicU32::new(1),
            owner_id: 0,
        }
    }

    /// Stamp the descriptor with the pool manager instance that owns the buffer
    pub fn with_owner(mut self, owner_id: u64) -> Self {
        self.owner_id = owner_id;
        self
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
        header.set_format_type(FORMAT_TYPE_INLINE);
        header.set_checksum(&payload);

        Self {
            header,
            payload: MessagePayload::Inline(payload),
        }
    }

    /// Create an inline message without computing a payload checksum
    pub fn new_inline_unchecked(topic_id: u32, sequence: u64, payload: Vec<u8>) -> Self {
        let mut header = MessageHeader::new(topic_id, sequence, payload.len() as u32);
        header.set_format_type(FORMAT_TYPE_INLINE);
        header.set_no_checksum();

        Self {
            header,
            payload: MessagePayload::Inline(payload),
        }
    }

    /// Create a new descriptor-based message
    pub fn new_descriptor(topic_id: u32, sequence: u64, descriptor: MessageDescriptor) -> Self {
        // For descriptor messages, the payload_length is the size of the descriptor itself,
        // not the size of the data it references
        let mut header = MessageHeader::new(
            topic_id,
            sequence,
            std::mem::size_of::<MessageDescriptor>() as u32,
        );
        header.set_format_type(FORMAT_TYPE_DESCRIPTOR);

        Self {
            header,
            payload: MessagePayload::Descriptor(descriptor),
        }
    }

    /// Get the total message size (header + payload)
    pub fn total_size(&self) -> usize {
        MessageHeader::SIZE
            + match &self.payload {
                MessagePayload::Inline(data) => data.len(),
                MessagePayload::Descriptor(_) => std::mem::size_of::<MessageDescriptor>(),
            }
    }
}
