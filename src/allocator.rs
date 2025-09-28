//! Memory allocators for shared memory regions

use std::{
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    error::{RenoirError, Result},
};

/// Trait for shared memory allocators
pub trait Allocator: Send + Sync + std::fmt::Debug {
    /// Allocate memory of the given size and alignment
    fn allocate(&self, size: usize, align: usize) -> Result<NonNull<u8>>;
    
    /// Deallocate previously allocated memory
    fn deallocate(&self, ptr: NonNull<u8>, size: usize, align: usize) -> Result<()>;
    
    /// Get the total size of the allocator
    fn total_size(&self) -> usize;
    
    /// Get the amount of used memory
    fn used_size(&self) -> usize;
    
    /// Get the amount of available memory
    fn available_size(&self) -> usize {
        self.total_size() - self.used_size()
    }
    
    /// Check if a pointer was allocated by this allocator
    fn owns(&self, ptr: NonNull<u8>) -> bool;
    
    /// Reset the allocator (if supported)
    fn reset(&self) -> Result<()>;
}

/// Simple bump allocator - allocates sequentially from a memory region
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
    /// Create a new bump allocator
    pub fn new(memory: &mut [u8]) -> Result<Self> {
        if memory.is_empty() {
            return Err(RenoirError::invalid_parameter("memory", "Memory region cannot be empty"));
        }
        
        let base_ptr = NonNull::new(memory.as_mut_ptr())
            .ok_or_else(|| RenoirError::memory("Invalid memory pointer"))?;
        
        Ok(Self {
            base_ptr,
            total_size: memory.len(),
            current_offset: AtomicUsize::new(0),
        })
    }
    
    /// Create from raw pointer and size (unsafe)
    pub unsafe fn from_raw(ptr: *mut u8, size: usize) -> Result<Self> {
        if ptr.is_null() || size == 0 {
            return Err(RenoirError::invalid_parameter("ptr", "Invalid pointer or size"));
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
}

impl Allocator for BumpAllocator {
    fn allocate(&self, size: usize, align: usize) -> Result<NonNull<u8>> {
        if size == 0 {
            return Err(RenoirError::invalid_parameter("size", "Size must be greater than 0"));
        }
        
        if !align.is_power_of_two() {
            return Err(RenoirError::invalid_parameter("align", "Alignment must be a power of 2"));
        }
        
        loop {
            let current = self.current_offset.load(Ordering::Acquire);
            let base_addr = self.base_ptr.as_ptr() as usize;
            let current_addr = base_addr + current;
            let aligned_addr = Self::align_up(current_addr, align);
            let new_offset = aligned_addr - base_addr + size;
            
            if new_offset > self.total_size {
                return Err(RenoirError::insufficient_space(size, self.total_size - current));
            }
            
            // Try to update the offset atomically
            match self.current_offset.compare_exchange_weak(
                current,
                new_offset,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let ptr = NonNull::new(aligned_addr as *mut u8)
                        .ok_or_else(|| RenoirError::memory("Failed to create pointer"))?;
                    return Ok(ptr);
                }
                Err(_) => {
                    // Retry with updated offset
                    continue;
                }
            }
        }
    }
    
    fn deallocate(&self, _ptr: NonNull<u8>, _size: usize, _align: usize) -> Result<()> {
        // Bump allocator doesn't support deallocation
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

/// Pool allocator - manages fixed-size blocks
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
    /// Create a new pool allocator
    pub fn new(memory: &mut [u8], block_size: usize) -> Result<Self> {
        if memory.is_empty() {
            return Err(RenoirError::invalid_parameter("memory", "Memory region cannot be empty"));
        }
        
        if block_size < std::mem::size_of::<usize>() {
            return Err(RenoirError::invalid_parameter(
                "block_size", 
                "Block size must be at least pointer size"
            ));
        }
        
        // Align block size to pointer boundary
        let aligned_block_size = Self::align_up(block_size, std::mem::align_of::<usize>());
        let total_blocks = memory.len() / aligned_block_size;
        
        if total_blocks == 0 {
            return Err(RenoirError::invalid_parameter(
                "block_size", 
                "Block size too large for memory region"
            ));
        }
        
        let base_ptr = NonNull::new(memory.as_mut_ptr())
            .ok_or_else(|| RenoirError::memory("Invalid memory pointer"))?;
        
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
    pub unsafe fn from_raw(ptr: *mut u8, size: usize, block_size: usize) -> Result<Self> {
        if ptr.is_null() || size == 0 {
            return Err(RenoirError::invalid_parameter("ptr", "Invalid pointer or size"));
        }
        
        let aligned_block_size = Self::align_up(block_size, std::mem::align_of::<usize>());
        let total_blocks = size / aligned_block_size;
        
        if total_blocks == 0 {
            return Err(RenoirError::invalid_parameter(
                "block_size", 
                "Block size too large for memory region"
            ));
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
    
    /// Initialize the free list by linking all blocks
    fn initialize_free_list(&self) {
        let base_addr = self.base_ptr.as_ptr() as usize;
        
        // Link all blocks in the free list
        for i in 0..self.total_blocks {
            let block_addr = base_addr + i * self.block_size;
            let next_index = if i + 1 < self.total_blocks { i + 1 } else { usize::MAX };
            
            unsafe {
                *(block_addr as *mut usize) = next_index;
            }
        }
    }
    
    /// Convert block index to memory address
    fn index_to_addr(&self, index: usize) -> usize {
        self.base_ptr.as_ptr() as usize + index * self.block_size
    }
    
    /// Convert memory address to block index
    fn addr_to_index(&self, addr: usize) -> usize {
        (addr - self.base_ptr.as_ptr() as usize) / self.block_size
    }
    
    /// Align a value up to the given alignment
    fn align_up(value: usize, align: usize) -> usize {
        (value + align - 1) & !(align - 1)
    }
}

impl Allocator for PoolAllocator {
    fn allocate(&self, size: usize, _align: usize) -> Result<NonNull<u8>> {
        if size == 0 {
            return Err(RenoirError::invalid_parameter("size", "Size must be greater than 0"));
        }
        
        if size > self.block_size {
            return Err(RenoirError::invalid_parameter(
                "size", 
                "Size exceeds block size"
            ));
        }
        
        loop {
            let current_head = self.free_head.load(Ordering::Acquire);
            
            if current_head == usize::MAX {
                return Err(RenoirError::insufficient_space(size, 0));
            }
            
            let current_addr = self.index_to_addr(current_head);
            let next_index = unsafe { *(current_addr as *const usize) };
            
            // Try to update the free head atomically
            match self.free_head.compare_exchange_weak(
                current_head,
                next_index,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.allocated_count.fetch_add(1, Ordering::Relaxed);
                    let ptr = NonNull::new(current_addr as *mut u8)
                        .ok_or_else(|| RenoirError::memory("Failed to create pointer"))?;
                    return Ok(ptr);
                }
                Err(_) => {
                    // Retry with updated head
                    continue;
                }
            }
        }
    }
    
    fn deallocate(&self, ptr: NonNull<u8>, _size: usize, _align: usize) -> Result<()> {
        if !self.owns(ptr) {
            return Err(RenoirError::invalid_parameter("ptr", "Pointer not owned by this allocator"));
        }
        
        let addr = ptr.as_ptr() as usize;
        let index = self.addr_to_index(addr);
        
        loop {
            let current_head = self.free_head.load(Ordering::Acquire);
            
            // Set the next pointer of this block to current head
            unsafe {
                *(addr as *mut usize) = current_head;
            }
            
            // Try to update the free head atomically
            match self.free_head.compare_exchange_weak(
                current_head,
                index,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.allocated_count.fetch_sub(1, Ordering::Relaxed);
                    return Ok(());
                }
                Err(_) => {
                    // Retry with updated head
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
        
        if ptr_addr < base_addr || ptr_addr >= end_addr {
            return false;
        }
        
        // Check if address is aligned to block boundary
        (ptr_addr - base_addr) % self.block_size == 0
    }
    
    fn reset(&self) -> Result<()> {
        self.initialize_free_list();
        self.free_head.store(0, Ordering::Release);
        self.allocated_count.store(0, Ordering::Release);
        Ok(())
    }
}

unsafe impl Send for PoolAllocator {}
unsafe impl Sync for PoolAllocator {}

/// Statistics for allocator performance monitoring
#[derive(Debug, Clone)]
pub struct AllocatorStats {
    pub total_size: usize,
    pub used_size: usize,
    pub available_size: usize,
    pub allocation_count: u64,
    pub deallocation_count: u64,
    pub fragmentation_ratio: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bump_allocator() {
        let mut memory = vec![0u8; 1024];
        let allocator = BumpAllocator::new(&mut memory).unwrap();
        
        assert_eq!(allocator.total_size(), 1024);
        assert_eq!(allocator.used_size(), 0);
        
        let ptr1 = allocator.allocate(64, 8).unwrap();
        assert!(allocator.owns(ptr1));
        assert!(allocator.used_size() >= 64);
        
        let ptr2 = allocator.allocate(128, 16).unwrap();
        assert!(allocator.owns(ptr2));
        
        // Bump allocator doesn't actually deallocate
        allocator.deallocate(ptr1, 64, 8).unwrap();
        
        allocator.reset().unwrap();
        assert_eq!(allocator.used_size(), 0);
    }
    
    #[test]
    fn test_pool_allocator() {
        let mut memory = vec![0u8; 1024];
        let allocator = PoolAllocator::new(&mut memory, 64).unwrap();
        
        assert_eq!(allocator.total_size(), 1024);
        assert_eq!(allocator.used_size(), 0);
        
        let ptr1 = allocator.allocate(32, 8).unwrap();
        assert!(allocator.owns(ptr1));
        assert_eq!(allocator.used_size(), 64);
        
        let ptr2 = allocator.allocate(64, 8).unwrap();
        assert!(allocator.owns(ptr2));
        assert_eq!(allocator.used_size(), 128);
        
        allocator.deallocate(ptr1, 32, 8).unwrap();
        assert_eq!(allocator.used_size(), 64);
        
        // Should be able to reuse the deallocated block
        let ptr3 = allocator.allocate(48, 8).unwrap();
        assert!(allocator.owns(ptr3));
        assert_eq!(allocator.used_size(), 128);
    }
    
    #[test]
    fn test_alignment() {
        assert_eq!(BumpAllocator::align_up(17, 8), 24);
        assert_eq!(BumpAllocator::align_up(16, 8), 16);
        assert_eq!(BumpAllocator::align_up(1, 64), 64);
    }
}