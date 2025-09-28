//! Topic-based messaging system for ROS2 integration
//! 
//! This module provides per-topic data structures optimized for different usage patterns:
//! - SPSC rings for single sensor writer → many readers
//! - MPMC rings for multiple writers/readers  
//! - Shared buffer pools for large objects referenced by descriptors

use std::{
    sync::atomic::{AtomicU64, AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::error::{RenoirError, Result};

/// Magic number for message header validation
pub const MESSAGE_MAGIC: u64 = 0x52454E4F49524D53; // "RENOMSG" in hex

/// Current message format version
pub const MESSAGE_VERSION: u32 = 1;

/// Maximum topic name length
pub const MAX_TOPIC_NAME_LENGTH: usize = 255;

/// Message header for all topic messages
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct MessageHeader {
    /// Magic number for validation (0x52454E4F49524D53)
    pub magic: u64,
    /// Message format version
    pub version: u32,
    /// Topic identifier hash
    pub topic_id: u32,
    /// Message sequence number (per-topic)
    pub sequence: u64,
    /// Timestamp in nanoseconds since UNIX epoch
    pub timestamp: u64,
    /// Payload length in bytes
    pub payload_length: u32,
    /// CRC32 checksum of payload
    pub checksum: u32,
    /// Reserved for future use (padding to 64-byte boundary)
    pub reserved: [u8; 20],
}

impl MessageHeader {
    /// Size of the message header in bytes
    pub const SIZE: usize = std::mem::size_of::<MessageHeader>();
    
    /// Create a new message header
    pub fn new(topic_id: u32, sequence: u64, payload_length: u32) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
            
        Self {
            magic: MESSAGE_MAGIC,
            version: MESSAGE_VERSION,
            topic_id,
            sequence,
            timestamp,
            payload_length,
            checksum: 0, // Will be calculated later
            reserved: [0; 20],
        }
    }
    
    /// Validate the message header
    pub fn validate(&self) -> Result<()> {
        if self.magic != MESSAGE_MAGIC {
            return Err(RenoirError::invalid_parameter(
                "magic", 
                "Invalid message magic number"
            ));
        }
        
        if self.version != MESSAGE_VERSION {
            return Err(RenoirError::invalid_parameter(
                "version", 
                "Unsupported message version"
            ));
        }
        
        Ok(())
    }
    
    /// Calculate and set the checksum for the payload
    pub fn set_checksum(&mut self, payload: &[u8]) {
        self.checksum = crc32fast::hash(payload);
    }
    
    /// Verify the checksum matches the payload
    pub fn verify_checksum(&self, payload: &[u8]) -> bool {
        self.checksum == crc32fast::hash(payload)
    }
}

/// Topic communication pattern
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicPattern {
    /// Single producer, single consumer (most common for sensors)
    SPSC,
    /// Single producer, multiple consumers (broadcast)
    SPMC,
    /// Multiple producers, single consumer (aggregation)  
    MPSC,
    /// Multiple producers, multiple consumers (rare, but needed)
    MPMC,
}

/// Topic configuration
#[derive(Debug, Clone)]
pub struct TopicConfig {
    /// Topic name (must be unique)
    pub name: String,
    /// Communication pattern
    pub pattern: TopicPattern,
    /// Ring buffer capacity (must be power of 2)
    pub ring_capacity: usize,
    /// Maximum message payload size
    pub max_payload_size: usize,
    /// Whether to use shared buffer pool for large messages
    pub use_shared_pool: bool,
    /// Threshold for using shared pool (bytes)
    pub shared_pool_threshold: usize,
    /// Enable eventfd notifications
    pub enable_notifications: bool,
    /// Quality of Service settings
    pub qos: TopicQoS,
}

impl Default for TopicConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            pattern: TopicPattern::SPSC,
            ring_capacity: 1024,
            max_payload_size: 64 * 1024, // 64KB
            use_shared_pool: false,
            shared_pool_threshold: 4096, // 4KB
            enable_notifications: true,
            qos: TopicQoS::default(),
        }
    }
}

/// Quality of Service settings for topics
#[derive(Debug, Clone)]
pub struct TopicQoS {
    /// Message durability (keep last N messages)
    pub durability: u32,
    /// Message reliability (best effort vs reliable)
    pub reliability: Reliability,
    /// History depth for late-joining subscribers
    pub history_depth: u32,
}

impl Default for TopicQoS {
    fn default() -> Self {
        Self {
            durability: 1,
            reliability: Reliability::BestEffort,
            history_depth: 10,
        }
    }
}

/// Message reliability levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reliability {
    /// Best effort delivery (may drop messages)
    BestEffort,
    /// Reliable delivery (waits for acknowledgment)
    Reliable,
}

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

/// Topic statistics
#[derive(Debug, Default)]
pub struct TopicStats {
    /// Total messages published
    pub messages_published: AtomicU64,
    /// Total messages consumed
    pub messages_consumed: AtomicU64,
    /// Messages dropped due to full buffer
    pub messages_dropped: AtomicU64,
    /// Current sequence number
    pub current_sequence: AtomicU64,
    /// Number of active publishers
    pub active_publishers: AtomicU32,
    /// Number of active subscribers
    pub active_subscribers: AtomicU32,
    /// Average message size in bytes
    pub avg_message_size: AtomicU64,
    /// Peak message rate (messages/second)
    pub peak_message_rate: AtomicU64,
}

impl TopicStats {
    /// Increment message published count and update sequence
    pub fn record_published(&self, message_size: usize) -> u64 {
        self.messages_published.fetch_add(1, Ordering::Relaxed);
        
        // Update running average message size
        let current_avg = self.avg_message_size.load(Ordering::Relaxed);
        let published_count = self.messages_published.load(Ordering::Relaxed);
        let new_avg = ((current_avg * (published_count - 1)) + message_size as u64) / published_count;
        self.avg_message_size.store(new_avg, Ordering::Relaxed);
        
        self.current_sequence.fetch_add(1, Ordering::SeqCst)
    }
    
    /// Increment message consumed count
    pub fn record_consumed(&self) {
        self.messages_consumed.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Increment dropped message count
    pub fn record_dropped(&self) {
        self.messages_dropped.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Add a publisher
    pub fn add_publisher(&self) -> u32 {
        self.active_publishers.fetch_add(1, Ordering::Relaxed) + 1
    }
    
    /// Remove a publisher
    pub fn remove_publisher(&self) -> u32 {
        self.active_publishers.fetch_sub(1, Ordering::Relaxed) - 1
    }
    
    /// Add a subscriber
    pub fn add_subscriber(&self) -> u32 {
        self.active_subscribers.fetch_add(1, Ordering::Relaxed) + 1
    }
    
    /// Remove a subscriber
    pub fn remove_subscriber(&self) -> u32 {
        self.active_subscribers.fetch_sub(1, Ordering::Relaxed) - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_header_creation() {
        let header = MessageHeader::new(42, 1337, 1024);
        
        // Copy values to locals to avoid packed field alignment issues
        let magic = header.magic;
        let version = header.version;
        let topic_id = header.topic_id;
        let sequence = header.sequence;
        let payload_length = header.payload_length;
        let timestamp = header.timestamp;
        
        assert_eq!(magic, MESSAGE_MAGIC);
        assert_eq!(version, MESSAGE_VERSION);
        assert_eq!(topic_id, 42);
        assert_eq!(sequence, 1337);
        assert_eq!(payload_length, 1024);
        assert!(timestamp > 0);
    }

    #[test]
    fn test_message_header_validation() {
        let mut header = MessageHeader::new(1, 1, 100);
        assert!(header.validate().is_ok());
        
        header.magic = 0xDEADBEEF;
        assert!(header.validate().is_err());
    }

    #[test]
    fn test_message_checksum() {
        let payload = b"Hello, Renoir!";
        let mut header = MessageHeader::new(1, 1, payload.len() as u32);
        
        header.set_checksum(payload);
        assert!(header.verify_checksum(payload));
        assert!(!header.verify_checksum(b"Different data"));
    }

    #[test]
    fn test_message_descriptor_ref_counting() {
        let desc = MessageDescriptor::new(1, 1, 1024);
        assert_eq!(desc.ref_count.load(Ordering::SeqCst), 1);
        
        desc.add_ref();
        assert_eq!(desc.ref_count.load(Ordering::SeqCst), 2);
        
        assert!(!desc.release()); // Still has references
        assert!(desc.release());  // Last reference
    }

    #[test]
    fn test_topic_stats() {
        let stats = TopicStats::default();
        
        let seq1 = stats.record_published(1024);
        let seq2 = stats.record_published(2048);
        
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(stats.messages_published.load(Ordering::Relaxed), 2);
        assert_eq!(stats.avg_message_size.load(Ordering::Relaxed), 1536); // (1024 + 2048) / 2
        
        stats.record_consumed();
        assert_eq!(stats.messages_consumed.load(Ordering::Relaxed), 1);
    }
}