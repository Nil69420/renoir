//! Buffer implementation for memory management

use std::{ptr::NonNull, slice, sync::Arc, time::SystemTime};

use crate::{
    allocators::Allocator,
    error::{RenoirError, Result},
};

/// A reference-counted buffer
#[derive(Debug, Clone)]
pub struct Buffer {
    data: NonNull<u8>,
    size: usize,
    capacity: usize,
    allocator: Arc<dyn Allocator>,
    sequence: u64,
    allocated_at: SystemTime,
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
        })
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

    /// Write data to the buffer
    pub fn write(&mut self, offset: usize, data: &[u8]) -> Result<()> {
        let Some(end) = offset.checked_add(data.len()) else {
            return Err(RenoirError::insufficient_space(data.len(), 0));
        };
        if end > self.capacity {
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

        if end > self.size {
            self.size = end;
        }

        Ok(())
    }

    /// Read data from the buffer
    pub fn read(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        let Some(end) = offset.checked_add(len) else {
            return Err(RenoirError::insufficient_space(len, 0));
        };
        if end > self.size {
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
