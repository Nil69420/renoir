//! Pool allocator implementation - fixed-size block allocation

use std::{
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::error::{RenoirError, Result};
use super::traits::Allocator;

/// Pool allocator for fixed-size blocks
/// 
/// This allocator manages a pool of fixed-size blocks and is extremely
/// efficient for allocating and deallocating objects of the same size.
#[derive(Debug)]
pub struct PoolAllocator {
    /// Base pointer to the memory region
    base_ptr: NonNull<u8>,
    /// Total size of the region
    total_size: usize,
    /// Size of each block
    block_size: usize,
    /// Total number of blocks
    total_blocks: usize,
    /// Free list head (atomic pointer to next free block)
    free_head: AtomicUsize,
    /// Number of allocated blocks
    allocated_count: AtomicUsize,
}

impl PoolAllocator {
    /// Create a new pool allocator from a mutable slice
    pub fn new(memory: &mut [u8], block_size: usize) -> Result<Self> {
        if memory.is_empty() {
            return Err(RenoirError::InvalidParameter {
                parameter: "memory".to_string(),
                message: "Memory region cannot be empty".to_string(),
            });
        }
        
        if block_size < std::mem::size_of::<usize>() {
            return Err(RenoirError::InvalidParameter {
                parameter: "block_size".to_string(),
                message: "Block size must be at least pointer size".to_string(),
            });
        }
        
        // Align block size to pointer boundary
        let aligned_block_size = Self::align_up(block_size, std::mem::align_of::<usize>());
        let total_blocks = memory.len() / aligned_block_size;
        
        if total_blocks == 0 {
            return Err(RenoirError::InvalidParameter {
                parameter: "block_size".to_string(),
                message: "Block size too large for memory region".to_string(),
            });
        }
        
        let base_ptr = NonNull::new(memory.as_mut_ptr())
            .ok_or_else(|| RenoirError::Memory {
                message: "Invalid memory pointer".to_string(),
            })?;
        
        let allocator = Self {
            base_ptr,
            total_size: memory.len(),
            block_size: aligned_block_size,
            total_blocks,
            free_head: AtomicUsize::new(0),
            allocated_count: AtomicUsize::new(0),
        };
        
        // Initialize free list
        allocator.initialize_free_list();
        
        Ok(allocator)
    }
    
    /// Create from raw pointer and size (unsafe)
    /// 
    /// # Safety
    /// - `ptr` must be valid for reads and writes for `size` bytes
    /// - The memory region must remain valid for the lifetime of the allocator
    pub unsafe fn from_raw(ptr: *mut u8, size: usize, block_size: usize) -> Result<Self> {
        if ptr.is_null() || size == 0 {
            return Err(RenoirError::InvalidParameter {
                parameter: "ptr".to_string(),
                message: "Invalid pointer or size".to_string(),
            });
        }
        
        let aligned_block_size = Self::align_up(block_size, std::mem::align_of::<usize>());
        let total_blocks = size / aligned_block_size;
        
        if total_blocks == 0 {
            return Err(RenoirError::InvalidParameter {
                parameter: "block_size".to_string(),
                message: "Block size too large for memory region".to_string(),
            });
        }
        
        let base_ptr = NonNull::new_unchecked(ptr);
        
        let allocator = Self {
            base_ptr,
            total_size: size,
            block_size: aligned_block_size,
            total_blocks,
            free_head: AtomicUsize::new(0),
            allocated_count: AtomicUsize::new(0),
        };
        
        allocator.initialize_free_list();
        
        Ok(allocator)
    }
    
    /// Get block size
    pub fn block_size(&self) -> usize {
        self.block_size
    }
    
    /// Get total number of blocks
    pub fn total_blocks(&self) -> usize {
        self.total_blocks
    }
    
    /// Get number of free blocks
    pub fn free_blocks(&self) -> usize {
        self.total_blocks - self.allocated_count.load(Ordering::Acquire)
    }
    
    /// Check if allocator is full
    pub fn is_full(&self) -> bool {
        self.allocated_count.load(Ordering::Acquire) >= self.total_blocks
    }
    
    /// Check if allocator is empty
    pub fn is_empty(&self) -> bool {
        self.allocated_count.load(Ordering::Acquire) == 0
    }
    
    /// Align a value up to the given alignment
    fn align_up(value: usize, align: usize) -> usize {
        (value + align - 1) & !(align - 1)
    }
    
    /// Initialize the free list by linking all blocks
    fn initialize_free_list(&self) {
        unsafe {
            let mut current_block = 0usize;
            
            // Link each block to the next one
            for i in 0..self.total_blocks - 1 {
                let block_ptr = self.base_ptr.as_ptr().add(i * self.block_size) as *mut usize;
                *block_ptr = current_block + self.block_size;
                current_block += self.block_size;
            }
            
            // Last block points to null (represented as usize::MAX)
            if self.total_blocks > 0 {
                let last_block_ptr = self.base_ptr.as_ptr()
                    .add((self.total_blocks - 1) * self.block_size) as *mut usize;
                *last_block_ptr = usize::MAX;
            }
        }
        
        self.free_head.store(0, Ordering::Release);
    }
    
    /// Validate that an offset corresponds to a valid block boundary
    fn is_valid_block_offset(&self, offset: usize) -> bool {
        offset % self.block_size == 0 && offset < self.total_size
    }
}

impl Allocator for PoolAllocator {
    fn allocate(&self, size: usize, align: usize) -> Result<NonNull<u8>> {
        // Pool allocator only supports allocations <= block_size
        if size > self.block_size {
            return Err(RenoirError::insufficient_space(size, self.block_size));
        }
        
        // Check alignment compatibility
        if align > std::mem::align_of::<usize>() && self.block_size % align != 0 {
            return Err(RenoirError::alignment(0, align));
        }
        
        loop {
            let head_offset = self.free_head.load(Ordering::Acquire);
            
            if head_offset == usize::MAX {
                // No free blocks available
                return Err(RenoirError::insufficient_space(size, 0));
            }
            
            // Get next free block from the current head
            let next_offset = unsafe {
                let head_ptr = self.base_ptr.as_ptr().add(head_offset) as *const usize;
                *head_ptr
            };
            
            // Try to update the head atomically
            match self.free_head.compare_exchange_weak(
                head_offset,
                next_offset,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Successfully took a block from the free list
                    self.allocated_count.fetch_add(1, Ordering::Relaxed);
                    
                    let ptr = NonNull::new(unsafe { self.base_ptr.as_ptr().add(head_offset) })
                        .ok_or_else(|| RenoirError::Memory {
                            message: "Failed to create pointer".to_string(),
                        })?;
                    
                    return Ok(ptr);
                }
                Err(_) => {
                    // Another thread updated the head, retry
                    std::hint::spin_loop();
                    continue;
                }
            }
        }
    }
    
    fn deallocate(&self, ptr: NonNull<u8>, _size: usize) -> Result<()> {
        let ptr_addr = ptr.as_ptr() as usize;
        let base_addr = self.base_ptr.as_ptr() as usize;
        
        if ptr_addr < base_addr {
            return Err(RenoirError::InvalidParameter {
                parameter: "ptr".to_string(),
                message: "Pointer not owned by this allocator".to_string(),
            });
        }
        
        let offset = ptr_addr - base_addr;
        
        if !self.is_valid_block_offset(offset) {
            return Err(RenoirError::InvalidParameter {
                parameter: "ptr".to_string(),
                message: "Pointer not aligned to block boundary".to_string(),
            });
        }
        
        // Add the block back to the free list
        loop {
            let current_head = self.free_head.load(Ordering::Acquire);
            
            // Set the next pointer of this block to point to current head
            unsafe {
                let block_ptr = ptr.as_ptr() as *mut usize;
                *block_ptr = current_head;
            }
            
            // Try to update the head to point to this block
            match self.free_head.compare_exchange_weak(
                current_head,
                offset,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.allocated_count.fetch_sub(1, Ordering::Relaxed);
                    return Ok(());
                }
                Err(_) => {
                    // Another thread updated the head, retry
                    std::hint::spin_loop();
                    continue;
                }
            }
        }
    }
    
    fn total_size(&self) -> usize {
        self.total_size
    }
    
    fn used_size(&self) -> usize {
        self.allocated_count.load(Ordering::Acquire) * self.block_size
    }
    
    fn owns(&self, ptr: NonNull<u8>) -> bool {
        let ptr_addr = ptr.as_ptr() as usize;
        let base_addr = self.base_ptr.as_ptr() as usize;
        let end_addr = base_addr + self.total_size;
        
        ptr_addr >= base_addr && ptr_addr < end_addr && self.is_valid_block_offset(ptr_addr - base_addr)
    }
    
    fn reset(&self) -> Result<()> {
        self.initialize_free_list();
        self.allocated_count.store(0, Ordering::Release);
        Ok(())
    }
}

unsafe impl Send for PoolAllocator {}
unsafe impl Sync for PoolAllocator {}