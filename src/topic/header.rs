//! Message header definitions and constants

use std::time::{SystemTime, UNIX_EPOCH};
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
    
    /// Set message format type in reserved bytes (backward compatible)
    pub fn set_format_type(&mut self, format_type: u8) {
        self.reserved[0] = format_type;
    }
    
    /// Get message format type from reserved bytes
    pub fn format_type(&self) -> u8 {
        self.reserved[0]
    }
    
    /// Set schema ID in reserved bytes (8 bytes starting at offset 1)
    pub fn set_schema_id(&mut self, schema_id: u64) {
        let bytes = schema_id.to_le_bytes();
        self.reserved[1..9].copy_from_slice(&bytes);
    }
    
    /// Get schema ID from reserved bytes
    pub fn schema_id(&self) -> Option<u64> {
        if self.reserved[1..9] == [0; 8] {
            None
        } else {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&self.reserved[1..9]);
            Some(u64::from_le_bytes(bytes))
        }
    }
    
    /// Set schema version in reserved bytes (4 bytes starting at offset 9)
    pub fn set_schema_version(&mut self, version: u32) {
        let bytes = version.to_le_bytes();
        self.reserved[9..13].copy_from_slice(&bytes);
    }
    
    /// Get schema version from reserved bytes
    pub fn schema_version(&self) -> Option<u32> {
        if self.reserved[9..13] == [0; 4] {
            None
        } else {
            let mut bytes = [0u8; 4];
            bytes.copy_from_slice(&self.reserved[9..13]);
            Some(u32::from_le_bytes(bytes))
        }
    }
    
    /// Set all format metadata at once
    pub fn set_format_metadata(&mut self, format_type: u8, schema_id: Option<u64>, schema_version: Option<u32>) {
        self.set_format_type(format_type);
        
        if let Some(id) = schema_id {
            self.set_schema_id(id);
        }
        
        if let Some(version) = schema_version {
            self.set_schema_version(version);
        }
    }
    
    /// Get all format metadata at once
    pub fn format_metadata(&self) -> (u8, Option<u64>, Option<u32>) {
        (
            self.format_type(),
            self.schema_id(),
            self.schema_version(),
        )
    }
    
    /// Check if header has format metadata set
    pub fn has_format_metadata(&self) -> bool {
        self.format_type() != 0 || self.schema_id().is_some()
    }
}