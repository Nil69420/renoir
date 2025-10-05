//! Bump allocator implementation - allocates sequentially from a memory region

use std::{
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::traits::Allocator;
use crate::error::{RenoirError, Result};

/// Simple bump allocator - allocates sequentially from a memory region
///
/// This allocator is extremely fast for allocation but cannot deallocate
/// individual allocations. All memory is reclaimed when the allocator is reset.
#[derive(Debug)]
pub struct BumpAllocator {
    /// Base pointer to the memory region
    base_ptr: NonNull<u8>,
    /// Total size of the region
    total_size: usize,
    /// Current offset (atomically updated)
    current_offset: AtomicUsize,
}

impl BumpAllocator {
    /// Create a new bump allocator from a mutable slice
    pub fn new(memory: &mut [u8]) -> Result<Self> {
        if memory.is_empty() {
            return Err(RenoirError::InvalidParameter {
                parameter: "memory".to_string(),
                message: "Memory region cannot be empty".to_string(),
            });
        }

        let base_ptr = NonNull::new(memory.as_mut_ptr()).ok_or_else(|| RenoirError::Memory {
            message: "Invalid memory pointer".to_string(),
        })?;

        Ok(Self {
            base_ptr,
            total_size: memory.len(),
            current_offset: AtomicUsize::new(0),
        })
    }

    /// Create from raw pointer and size (unsafe)
    ///
    /// # Safety
    /// - `ptr` must be valid for reads and writes for `size` bytes
    /// - The memory region must remain valid for the lifetime of the allocator
    pub unsafe fn from_raw(ptr: *mut u8, size: usize) -> Result<Self> {
        if ptr.is_null() || size == 0 {
            return Err(RenoirError::InvalidParameter {
                parameter: "ptr".to_string(),
                message: "Invalid pointer or size".to_string(),
            });
        }

        let base_ptr = NonNull::new_unchecked(ptr);

        Ok(Self {
            base_ptr,
            total_size: size,
            current_offset: AtomicUsize::new(0),
        })
    }

    /// Align a value up to the given alignment
    fn align_up(value: usize, align: usize) -> usize {
        (value + align - 1) & !(align - 1)
    }

    /// Get current position in the allocation region
    pub fn current_position(&self) -> usize {
        self.current_offset.load(Ordering::Acquire)
    }

    /// Check if a specific size can be allocated
    pub fn can_allocate_size(&self, size: usize, align: usize) -> bool {
        let current = self.current_offset.load(Ordering::Acquire);
        let base_addr = self.base_ptr.as_ptr() as usize;
        let current_addr = base_addr + current;
        let aligned_addr = Self::align_up(current_addr, align);
        let required_offset = aligned_addr - base_addr + size;

        required_offset <= self.total_size
    }
}

impl Allocator for BumpAllocator {
    fn allocate(&self, size: usize, align: usize) -> Result<NonNull<u8>> {
        if size == 0 {
            return Err(RenoirError::InvalidParameter {
                parameter: "size".to_string(),
                message: "Size must be greater than 0".to_string(),
            });
        }

        if !align.is_power_of_two() {
            return Err(RenoirError::InvalidParameter {
                parameter: "align".to_string(),
                message: "Alignment must be a power of 2".to_string(),
            });
        }

        loop {
            let current = self.current_offset.load(Ordering::Acquire);
            let base_addr = self.base_ptr.as_ptr() as usize;
            let current_addr = base_addr + current;
            let aligned_addr = Self::align_up(current_addr, align);
            let new_offset = aligned_addr - base_addr + size;

            if new_offset > self.total_size {
                return Err(RenoirError::insufficient_space(
                    size,
                    self.total_size - current,
                ));
            }

            // Try to update the offset atomically
            match self.current_offset.compare_exchange_weak(
                current,
                new_offset,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let ptr = NonNull::new(aligned_addr as *mut u8).ok_or_else(|| {
                        RenoirError::Memory {
                            message: "Failed to create pointer".to_string(),
                        }
                    })?;
                    return Ok(ptr);
                }
                Err(_) => {
                    // Another thread updated the offset, retry
                    std::hint::spin_loop();
                    continue;
                }
            }
        }
    }

    fn deallocate(&self, _ptr: NonNull<u8>, _size: usize) -> Result<()> {
        // Bump allocator doesn't support individual deallocation
        Ok(())
    }

    fn total_size(&self) -> usize {
        self.total_size
    }

    fn used_size(&self) -> usize {
        self.current_offset.load(Ordering::Acquire)
    }

    fn owns(&self, ptr: NonNull<u8>) -> bool {
        let ptr_addr = ptr.as_ptr() as usize;
        let base_addr = self.base_ptr.as_ptr() as usize;
        let end_addr = base_addr + self.total_size;

        ptr_addr >= base_addr && ptr_addr < end_addr
    }

    fn reset(&self) -> Result<()> {
        self.current_offset.store(0, Ordering::Release);
        Ok(())
    }
}

unsafe impl Send for BumpAllocator {}
unsafe impl Sync for BumpAllocator {}
