//! Chunking system for extremely large payloads that exceed buffer pool limits
//!
//! Handles splitting large messages (like high-resolution images) into multiple
//! fixed-size chunks with automatic reassembly on the reader side.

use super::blob::BlobHeader;
use crate::error::{RenoirError, Result};
use crate::shared_pools::{PoolId, SharedBufferPoolManager};
use crate::topic::MessageDescriptor;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Maximum chunk size (configurable, typically 64KB-4MB)
pub const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024; // 1MB

/// Maximum number of chunks per message
pub const MAX_CHUNKS_PER_MESSAGE: u32 = 1024;

/// Chunk descriptor for individual chunks
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ChunkDescriptor {
    /// Sequence ID (matches BlobHeader sequence_id)
    pub sequence_id: u64,
    /// Chunk index (0-based)
    pub chunk_index: u32,
    /// Total number of chunks
    pub total_chunks: u32,
    /// Size of this chunk
    pub chunk_size: u32,
    /// Offset in the original payload
    pub payload_offset: u64,
    /// CRC32 checksum of this chunk
    pub checksum: u32,
    /// Reserved for alignment
    pub reserved: [u8; 4],
}

impl ChunkDescriptor {
    /// Size of chunk descriptor in bytes
    pub const SIZE: usize = std::mem::size_of::<ChunkDescriptor>();

    /// Create a new chunk descriptor
    pub fn new(
        sequence_id: u64,
        chunk_index: u32,
        total_chunks: u32,
        chunk_size: u32,
        payload_offset: u64,
    ) -> Self {
        Self {
            sequence_id,
            chunk_index,
            total_chunks,
            chunk_size,
            payload_offset,
            checksum: 0,
            reserved: [0; 4],
        }
    }

    /// Set checksum for the chunk data
    pub fn set_checksum(&mut self, chunk_data: &[u8]) {
        self.checksum = crc32fast::hash(chunk_data);
    }

    /// Verify chunk checksum
    pub fn verify_checksum(&self, chunk_data: &[u8]) -> bool {
        self.checksum == crc32fast::hash(chunk_data)
    }

    /// Check if this is the last chunk
    pub fn is_last_chunk(&self) -> bool {
        self.chunk_index + 1 == self.total_chunks
    }
}

/// Strategy for chunking large payloads
#[derive(Debug, Clone)]
pub enum ChunkingStrategy {
    /// Fixed size chunks
    FixedSize(usize),
    /// Adaptive size based on content (future extension)
    Adaptive { min_size: usize, max_size: usize },
}

impl Default for ChunkingStrategy {
    fn default() -> Self {
        Self::FixedSize(DEFAULT_CHUNK_SIZE)
    }
}

/// Incomplete message being reassembled
#[derive(Debug)]
struct IncompleteMessage {
    /// Expected number of chunks
    total_chunks: u32,
    /// Received chunks (chunk_index -> (data, descriptor))
    chunks: HashMap<u32, (Vec<u8>, ChunkDescriptor)>,
    /// Creation timestamp for cleanup
    created_at: SystemTime,
    /// Original blob header
    #[allow(dead_code)]
    blob_header: Option<BlobHeader>,
}

impl IncompleteMessage {
    fn new(total_chunks: u32) -> Self {
        Self {
            total_chunks,
            chunks: HashMap::new(),
            created_at: SystemTime::now(),
            blob_header: None,
        }
    }

    /// Add a chunk to the incomplete message
    fn add_chunk(&mut self, chunk_data: Vec<u8>, descriptor: ChunkDescriptor) -> Result<()> {
        if descriptor.chunk_index >= self.total_chunks {
            return Err(RenoirError::invalid_parameter(
                "chunk_index",
                "Chunk index exceeds total chunks",
            ));
        }

        if !descriptor.verify_checksum(&chunk_data) {
            return Err(RenoirError::invalid_parameter(
                "checksum",
                "Chunk checksum validation failed",
            ));
        }

        self.chunks
            .insert(descriptor.chunk_index, (chunk_data, descriptor));
        Ok(())
    }

    /// Check if message is complete
    fn is_complete(&self) -> bool {
        self.chunks.len() == self.total_chunks as usize
    }

    /// Reassemble the complete payload
    fn reassemble(&self) -> Result<Vec<u8>> {
        if !self.is_complete() {
            return Err(RenoirError::invalid_parameter(
                "chunks",
                "Cannot reassemble incomplete message",
            ));
        }

        // Calculate total size
        let total_size: usize = self.chunks.values().map(|(data, _)| data.len()).sum();

        let mut result = Vec::with_capacity(total_size);

        // Reassemble chunks in order
        for chunk_index in 0..self.total_chunks {
            if let Some((data, _)) = self.chunks.get(&chunk_index) {
                result.extend_from_slice(data);
            } else {
                return Err(RenoirError::invalid_parameter(
                    "chunk_order",
                    "Missing chunk in sequence",
                ));
            }
        }

        Ok(result)
    }

    /// Check if this message has expired
    fn is_expired(&self, timeout: Duration) -> bool {
        self.created_at.elapsed().unwrap_or(timeout) >= timeout
    }
}

/// Chunk manager for handling large message chunking and reassembly
#[derive(Debug)]
pub struct ChunkManager {
    /// Buffer pool manager for storing chunks
    pool_manager: Arc<SharedBufferPoolManager>,
    /// Chunking strategy
    strategy: ChunkingStrategy,
    /// Incomplete messages being reassembled (sequence_id -> IncompleteMessage)
    incomplete_messages: Arc<Mutex<HashMap<u64, IncompleteMessage>>>,
    /// Reassembly timeout
    reassembly_timeout: Duration,
}

impl ChunkManager {
    /// Create a new chunk manager
    pub fn new(pool_manager: Arc<SharedBufferPoolManager>, strategy: ChunkingStrategy) -> Self {
        Self {
            pool_manager,
            strategy,
            incomplete_messages: Arc::new(Mutex::new(HashMap::new())),
            reassembly_timeout: Duration::from_secs(30), // 30 second timeout
        }
    }

    /// Set reassembly timeout
    pub fn set_reassembly_timeout(&mut self, timeout: Duration) {
        self.reassembly_timeout = timeout;
    }

    /// Check if payload needs chunking
    pub fn needs_chunking(&self, payload_size: usize, max_buffer_size: usize) -> bool {
        payload_size > max_buffer_size
    }

    /// Split payload into chunks and store in buffer pool
    pub fn chunk_payload(
        &self,
        payload: &[u8],
        blob_header: &BlobHeader,
        pool_id: PoolId,
    ) -> Result<Vec<MessageDescriptor>> {
        let chunk_size = match &self.strategy {
            ChunkingStrategy::FixedSize(size) => *size,
            ChunkingStrategy::Adaptive {
                min_size,
                max_size: _,
            } => {
                // Simple adaptive strategy: use min_size for now
                *min_size
            }
        };

        let total_chunks = ((payload.len() + chunk_size - 1) / chunk_size) as u32;

        if total_chunks > MAX_CHUNKS_PER_MESSAGE {
            return Err(RenoirError::invalid_parameter(
                "payload_size",
                "Payload too large for chunking system",
            ));
        }

        let mut descriptors = Vec::with_capacity(total_chunks as usize);

        for chunk_index in 0..total_chunks {
            let start_offset = (chunk_index as usize) * chunk_size;
            let end_offset = std::cmp::min(start_offset + chunk_size, payload.len());
            let chunk_data = &payload[start_offset..end_offset];

            // Create chunk descriptor
            let mut chunk_desc = ChunkDescriptor::new(
                blob_header.sequence_id,
                chunk_index,
                total_chunks,
                chunk_data.len() as u32,
                start_offset as u64,
            );
            chunk_desc.set_checksum(chunk_data);

            // Get buffer from pool
            let buffer_desc = self.pool_manager.get_buffer(pool_id)?;
            let buffer_data = self.pool_manager.get_buffer_data(&buffer_desc)?;

            // Write chunk descriptor + data to buffer
            unsafe {
                let buffer_ptr = buffer_data.as_mut_ptr();

                // Write chunk descriptor first
                std::ptr::copy_nonoverlapping(
                    &chunk_desc as *const ChunkDescriptor as *const u8,
                    buffer_ptr,
                    ChunkDescriptor::SIZE,
                );

                // Write chunk data after descriptor
                std::ptr::copy_nonoverlapping(
                    chunk_data.as_ptr(),
                    buffer_ptr.add(ChunkDescriptor::SIZE),
                    chunk_data.len(),
                );
            }

            descriptors.push(buffer_desc);
        }

        Ok(descriptors)
    }

    /// Process an incoming chunk
    pub fn process_chunk(&self, chunk_buffer: &[u8]) -> Result<Option<Vec<u8>>> {
        if chunk_buffer.len() < ChunkDescriptor::SIZE {
            return Err(RenoirError::invalid_parameter(
                "chunk_size",
                "Buffer too small for chunk descriptor",
            ));
        }

        // Read chunk descriptor
        let chunk_desc: ChunkDescriptor =
            unsafe { std::ptr::read_unaligned(chunk_buffer.as_ptr() as *const ChunkDescriptor) };

        // Extract chunk data
        let chunk_data_offset = ChunkDescriptor::SIZE;
        let chunk_data =
            &chunk_buffer[chunk_data_offset..chunk_data_offset + chunk_desc.chunk_size as usize];

        // Verify chunk
        if !chunk_desc.verify_checksum(chunk_data) {
            return Err(RenoirError::invalid_parameter(
                "checksum",
                "Chunk checksum validation failed",
            ));
        }

        let mut incomplete = self.incomplete_messages.lock().unwrap();

        // Get or create incomplete message
        let incomplete_msg = incomplete
            .entry(chunk_desc.sequence_id)
            .or_insert_with(|| IncompleteMessage::new(chunk_desc.total_chunks));

        // Add chunk to incomplete message
        incomplete_msg.add_chunk(chunk_data.to_vec(), chunk_desc)?;

        // Check if message is complete
        if incomplete_msg.is_complete() {
            // Reassemble and return complete message
            let complete_payload = incomplete_msg.reassemble()?;
            // Copy sequence_id to avoid referencing packed field
            let sequence_id = chunk_desc.sequence_id;
            incomplete.remove(&sequence_id);
            Ok(Some(complete_payload))
        } else {
            Ok(None)
        }
    }

    /// Clean up expired incomplete messages
    pub fn cleanup_expired(&self) -> usize {
        let mut incomplete = self.incomplete_messages.lock().unwrap();
        let initial_count = incomplete.len();

        incomplete.retain(|_, msg| !msg.is_expired(self.reassembly_timeout));

        initial_count - incomplete.len()
    }

    /// Get statistics about incomplete messages
    pub fn get_statistics(&self) -> ChunkManagerStats {
        let incomplete = self.incomplete_messages.lock().unwrap();

        let incomplete_count = incomplete.len();
        let total_chunks_waiting: u32 =
            incomplete.values().map(|msg| msg.chunks.len() as u32).sum();
        let oldest_message_age = incomplete
            .values()
            .map(|msg| msg.created_at.elapsed().unwrap_or_default())
            .max()
            .unwrap_or_default();

        ChunkManagerStats {
            incomplete_messages: incomplete_count,
            total_chunks_waiting,
            oldest_message_age,
        }
    }
}

/// Statistics for chunk manager
#[derive(Debug, Clone)]
pub struct ChunkManagerStats {
    /// Number of incomplete messages
    pub incomplete_messages: usize,
    /// Total chunks waiting for reassembly
    pub total_chunks_waiting: u32,
    /// Age of oldest incomplete message
    pub oldest_message_age: Duration,
}
