//! Blob header and management for large payloads
//! 
//! Provides structured headers for data stored in shared buffer pools,
//! including magic numbers, versioning, and integrity validation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::error::{RenoirError, Result};
use crate::shared_pools::{PoolId, BufferHandle};

/// Magic number for blob validation
pub const BLOB_MAGIC: u64 = 0x52454E4F49424C42; // "RENOIBLB" in hex

/// Current blob format version
pub const BLOB_VERSION: u32 = 1;

/// Blob header for data stored in shared buffer pools
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BlobHeader {
    /// Magic number for validation
    pub magic: u64,
    /// Blob format version
    pub version: u32,
    /// Type of content (ROS2 message type)
    pub content_type: u32,
    /// Creation timestamp (nanoseconds since UNIX epoch)
    pub timestamp: u64,
    /// Total payload size in bytes (excluding header)
    pub payload_size: u64,
    /// CRC32 checksum of payload data
    pub checksum: u32,
    /// Number of chunks (1 for single blob, >1 for chunked)
    pub chunk_count: u32,
    /// Sequence number for chunked data
    pub sequence_id: u64,
    /// Compression type (0 = none, 1 = lz4, 2 = zstd)
    pub compression: u32,
    /// Original size before compression (0 if uncompressed)
    pub original_size: u64,
    /// Reserved for future use (padding to 64-byte boundary)
    pub reserved: [u8; 4],
}

impl BlobHeader {
    /// Size of the blob header in bytes
    pub const SIZE: usize = std::mem::size_of::<BlobHeader>();
    
    /// Create a new blob header
    pub fn new(content_type: u32, payload_size: u64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
            
        Self {
            magic: BLOB_MAGIC,
            version: BLOB_VERSION,
            content_type,
            timestamp,
            payload_size,
            checksum: 0, // Will be calculated later
            chunk_count: 1,
            sequence_id: 0,
            compression: 0,
            original_size: 0,
            reserved: [0; 4],
        }
    }
    
    /// Validate the blob header
    pub fn validate(&self) -> Result<()> {
        if self.magic != BLOB_MAGIC {
            return Err(RenoirError::invalid_parameter(
                "magic", 
                "Invalid blob magic number"
            ));
        }
        
        if self.version != BLOB_VERSION {
            return Err(RenoirError::invalid_parameter(
                "version", 
                "Unsupported blob version"
            ));
        }
        
        if self.payload_size == 0 {
            return Err(RenoirError::invalid_parameter(
                "payload_size", 
                "Blob payload size cannot be zero"
            ));
        }
        
        Ok(())
    }
    
    /// Calculate and set checksum for the payload
    pub fn set_checksum(&mut self, payload: &[u8]) {
        self.checksum = crc32fast::hash(payload);
    }
    
    /// Verify checksum matches the payload
    pub fn verify_checksum(&self, payload: &[u8]) -> bool {
        self.checksum == crc32fast::hash(payload)
    }
    
    /// Set chunking information
    pub fn set_chunking(&mut self, chunk_count: u32, sequence_id: u64) {
        self.chunk_count = chunk_count;
        self.sequence_id = sequence_id;
    }
    
    /// Set compression information
    pub fn set_compression(&mut self, compression_type: u32, original_size: u64) {
        self.compression = compression_type;
        self.original_size = original_size;
    }
    
    /// Check if the blob is chunked
    pub fn is_chunked(&self) -> bool {
        self.chunk_count > 1
    }
    
    /// Check if the blob is compressed
    pub fn is_compressed(&self) -> bool {
        self.compression > 0
    }
}

/// Extended descriptor for large payloads with blob headers
#[derive(Debug)]
pub struct BlobDescriptor {
    /// Pool ID where the blob is stored
    pub pool_id: PoolId,
    /// Buffer handle within the pool
    pub buffer_handle: BufferHandle,
    /// Total size including header
    pub total_size: u64,
    /// Blob header (cached for quick access)
    pub header: BlobHeader,
    /// Reference count for memory management
    pub ref_count: AtomicU64,
}

impl Clone for BlobDescriptor {
    fn clone(&self) -> Self {
        Self {
            pool_id: self.pool_id,
            buffer_handle: self.buffer_handle,
            total_size: self.total_size,
            header: self.header,
            ref_count: AtomicU64::new(self.ref_count.load(std::sync::atomic::Ordering::Relaxed)),
        }
    }
}

impl BlobDescriptor {
    /// Create a new blob descriptor
    pub fn new(
        pool_id: PoolId, 
        buffer_handle: BufferHandle, 
        header: BlobHeader
    ) -> Self {
        let total_size = BlobHeader::SIZE as u64 + header.payload_size;
        
        Self {
            pool_id,
            buffer_handle,
            total_size,
            header,
            ref_count: AtomicU64::new(1),
        }
    }
    
    /// Add reference count
    pub fn add_ref(&self) -> u64 {
        self.ref_count.fetch_add(1, Ordering::SeqCst)
    }
    
    /// Release reference, returns true if should be reclaimed
    pub fn release(&self) -> bool {
        self.ref_count.fetch_sub(1, Ordering::SeqCst) == 1
    }
    
    /// Get current reference count
    pub fn ref_count(&self) -> u64 {
        self.ref_count.load(Ordering::SeqCst)
    }
    
    /// Get payload size (excluding header)
    pub fn payload_size(&self) -> u64 {
        self.header.payload_size
    }
    
    /// Get content type
    pub fn content_type(&self) -> u32 {
        self.header.content_type
    }
}

/// Blob manager for handling large payloads with headers
#[derive(Debug)]
pub struct BlobManager {
    /// Next sequence ID for chunked data
    next_sequence: AtomicU64,
}

impl BlobManager {
    /// Create a new blob manager
    pub fn new() -> Self {
        Self {
            next_sequence: AtomicU64::new(1),
        }
    }
    
    /// Get next sequence ID for chunked data
    pub fn next_sequence_id(&self) -> u64 {
        self.next_sequence.fetch_add(1, Ordering::SeqCst)
    }
    
    /// Create a blob header for the given content
    pub fn create_header(&self, content_type: u32, payload: &[u8]) -> BlobHeader {
        let mut header = BlobHeader::new(content_type, payload.len() as u64);
        header.set_checksum(payload);
        header
    }
    
    /// Create a blob header for chunked content
    pub fn create_chunked_header(
        &self,
        content_type: u32,
        total_payload_size: u64,
        chunk_count: u32
    ) -> BlobHeader {
        let mut header = BlobHeader::new(content_type, total_payload_size);
        let sequence_id = self.next_sequence_id();
        header.set_chunking(chunk_count, sequence_id);
        header
    }
    
    /// Validate blob integrity
    pub fn validate_blob(&self, header: &BlobHeader, payload: &[u8]) -> Result<()> {
        header.validate()?;
        
        if header.payload_size != payload.len() as u64 {
            return Err(RenoirError::invalid_parameter(
                "payload_size",
                "Header payload size doesn't match actual payload"
            ));
        }
        
        if !header.verify_checksum(payload) {
            return Err(RenoirError::invalid_parameter(
                "checksum",
                "Blob checksum validation failed"
            ));
        }
        
        Ok(())
    }
}

impl Default for BlobManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Content type constants for common ROS2 message types
pub mod content_types {
    /// sensor_msgs/Image
    pub const IMAGE: u32 = 0x494D4147; // "IMAG"
    
    /// sensor_msgs/PointCloud2
    pub const POINT_CLOUD: u32 = 0x50434C44; // "PCLD"
    
    /// sensor_msgs/LaserScan
    pub const LASER_SCAN: u32 = 0x4C53434E; // "LSCN"
    
    /// nav_msgs/OccupancyGrid
    pub const OCCUPANCY_GRID: u32 = 0x4F475244; // "OGRD"
    
    /// geometry_msgs/PolygonStamped
    pub const POLYGON: u32 = 0x504F4C59; // "POLY"
    
    /// Custom binary data
    pub const CUSTOM_BINARY: u32 = 0x43555354; // "CUST"
}

