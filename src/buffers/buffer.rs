//! Buffer implementation for memory management

use std::{collections::HashMap, ptr::NonNull, slice, sync::Arc, time::SystemTime};

use crate::{
    allocators::Allocator,
    error::{RenoirError, Result},
};

/// A reference-counted buffer with metadata
#[derive(Debug, Clone)]
#[allow(clippy::box_collection)]
pub struct Buffer {
    data: NonNull<u8>,
    size: usize,
    capacity: usize,
    allocator: Arc<dyn Allocator>,
    sequence: u64,
    allocated_at: SystemTime,
    /// Lazily allocated — None until first `set_metadata` call.
    metadata: Option<Box<HashMap<String, String>>>,
}

impl Buffer {
    /// Create a new buffer from an allocator
    pub fn new(allocator: Arc<dyn Allocator>, size: usize, align: usize) -> Result<Self> {
        if size == 0 {
            return Err(RenoirError::invalid_parameter(
                "size",
                "Buffer size cannot be zero",
            ));
        }

        let capacity = size;
        let data = allocator.allocate(capacity, align)?;

        Ok(Self {
            data,
            size,
            capacity,
            allocator,
            sequence: 0,
            allocated_at: SystemTime::now(),
            metadata: None,
        })
    }

    /// Create a buffer from existing data pointer (unsafe)
    ///
    /// # Safety
    /// The caller must ensure that:
    /// - `data` points to valid memory of at least `capacity` bytes
    /// - The memory is properly aligned
    /// - The allocator can safely deallocate this memory
    pub unsafe fn from_raw(
        data: NonNull<u8>,
        size: usize,
        capacity: usize,
        allocator: Arc<dyn Allocator>,
    ) -> Self {
        Self {
            data,
            size,
            capacity,
            allocator,
            sequence: 0,
            allocated_at: SystemTime::now(),
            metadata: None,
        }
    }

    /// Get a raw pointer to the buffer data
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// Get a mutable raw pointer to the buffer data
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.data.as_ptr()
    }

    /// Get the buffer as a byte slice
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `data` is valid for `size` bytes, allocated by our allocator.
        unsafe { slice::from_raw_parts(self.data.as_ptr(), self.size) }
    }

    /// Get the buffer as a mutable byte slice
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: `data` is valid for `size` bytes, and we have `&mut self` ensuring exclusive access.
        unsafe { slice::from_raw_parts_mut(self.data.as_ptr(), self.size) }
    }

    /// Get the size of the buffer
    pub fn size(&self) -> usize {
        self.size
    }

    /// Get the capacity of the buffer
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the sequence number
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Set the sequence number
    pub fn set_sequence(&mut self, sequence: u64) {
        self.sequence = sequence;
    }

    /// Get the allocation timestamp
    pub fn allocated_at(&self) -> SystemTime {
        self.allocated_at
    }

    /// Resize the buffer (may move data)
    pub fn resize(&mut self, new_size: usize) -> Result<()> {
        if new_size > self.capacity {
            return Err(RenoirError::insufficient_space(new_size, self.capacity));
        }
        self.size = new_size;
        Ok(())
    }

    /// Clear the buffer (set size to 0)
    pub fn clear(&mut self) {
        self.size = 0;
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Get buffer alignment
    pub fn alignment(&self) -> usize {
        self.allocator.alignment()
    }

    /// Add metadata to the buffer
    pub fn set_metadata(&mut self, key: String, value: String) {
        self.metadata
            .get_or_insert_with(|| Box::new(HashMap::new()))
            .insert(key, value);
    }

    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.as_ref().and_then(|m| m.get(key))
    }

    pub fn metadata(&self) -> &HashMap<String, String> {
        static EMPTY: std::sync::LazyLock<HashMap<String, String>> =
            std::sync::LazyLock::new(HashMap::new);
        self.metadata.as_deref().unwrap_or(&EMPTY)
    }

    pub fn metadata_opt(&self) -> Option<&HashMap<String, String>> {
        self.metadata.as_deref()
    }

    /// Write data to the buffer
    pub fn write(&mut self, offset: usize, data: &[u8]) -> Result<()> {
        if offset + data.len() > self.capacity {
            let available = self.capacity.saturating_sub(offset);
            return Err(RenoirError::insufficient_space(data.len(), available));
        }

        // SAFETY: Bounds checked above; `offset + data.len() <= capacity` and `data` is a valid slice.
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.data.as_ptr().add(offset),
                data.len(),
            );
        }

        if offset + data.len() > self.size {
            self.size = offset + data.len();
        }

        Ok(())
    }

    /// Read data from the buffer
    pub fn read(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        if offset + len > self.size {
            let available = self.size.saturating_sub(offset);
            return Err(RenoirError::insufficient_space(len, available));
        }

        let mut data = vec![0u8; len];
        // SAFETY: Bounds checked above; `offset + len <= size` and `data` has `len` bytes allocated.
        unsafe {
            std::ptr::copy_nonoverlapping(self.data.as_ptr().add(offset), data.as_mut_ptr(), len);
        }

        Ok(data)
    }

    /// Zero the buffer contents
    pub fn zero(&mut self) {
        // SAFETY: `data` is valid for `capacity` bytes and we have exclusive access via `&mut self`.
        unsafe {
            std::ptr::write_bytes(self.data.as_ptr(), 0, self.capacity);
        }
    }

    /// Get a reference to the allocator
    pub fn allocator(&self) -> &Arc<dyn Allocator> {
        &self.allocator
    }
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Drop for Buffer {
    fn drop(&mut self) {
        // The allocator will handle deallocation when the Arc is dropped
        if let Err(e) = self.allocator.deallocate(self.data, self.capacity) {
            log::warn!("Failed to deallocate buffer: {:?}", e);
        }
    }
}

impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsMut<[u8]> for Buffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}
