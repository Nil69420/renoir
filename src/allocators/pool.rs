//! Pool allocator implementation - fixed-size block allocation

use std::{
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::traits::Allocator;
use crate::error::{RenoirError, Result};

const MAX_SHARDS: usize = 8;

/// Pool allocator for fixed-size blocks with sharded free lists.
///
/// Blocks are distributed across multiple shards to reduce atomic contention.
/// Each thread selects a shard via its thread ID; if the local shard is empty,
/// it steals from other shards round-robin.
#[derive(Debug)]
pub struct PoolAllocator {
    base_ptr: NonNull<u8>,
    total_size: usize,
    block_size: usize,
    total_blocks: usize,
    num_shards: usize,
    shard_heads: [AtomicUsize; MAX_SHARDS],
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

        let aligned_block_size = Self::align_up(block_size, std::mem::align_of::<usize>());
        let total_blocks = memory.len() / aligned_block_size;

        if total_blocks == 0 {
            return Err(RenoirError::InvalidParameter {
                parameter: "block_size".to_string(),
                message: "Block size too large for memory region".to_string(),
            });
        }

        let base_ptr = NonNull::new(memory.as_mut_ptr()).ok_or_else(|| RenoirError::Memory {
            message: "Invalid memory pointer".to_string(),
        })?;

        let num_shards = total_blocks.min(MAX_SHARDS);

        let allocator = Self {
            base_ptr,
            total_size: memory.len(),
            block_size: aligned_block_size,
            total_blocks,
            num_shards,
            shard_heads: std::array::from_fn(|_| AtomicUsize::new(usize::MAX)),
            allocated_count: AtomicUsize::new(0),
        };

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
        let num_shards = total_blocks.min(MAX_SHARDS);

        let allocator = Self {
            base_ptr,
            total_size: size,
            block_size: aligned_block_size,
            total_blocks,
            num_shards,
            shard_heads: std::array::from_fn(|_| AtomicUsize::new(usize::MAX)),
            allocated_count: AtomicUsize::new(0),
        };

        allocator.initialize_free_list();

        Ok(allocator)
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    pub fn total_blocks(&self) -> usize {
        self.total_blocks
    }

    pub fn free_blocks(&self) -> usize {
        self.total_blocks - self.allocated_count.load(Ordering::Acquire)
    }

    pub fn is_full(&self) -> bool {
        self.allocated_count.load(Ordering::Acquire) >= self.total_blocks
    }

    pub fn is_empty(&self) -> bool {
        self.allocated_count.load(Ordering::Acquire) == 0
    }

    fn align_up(value: usize, align: usize) -> usize {
        (value + align - 1) & !(align - 1)
    }

    /// Initialize the free list by distributing blocks across shards round-robin.
    fn initialize_free_list(&self) {
        // Assign each block to a shard: block i → shard (i % num_shards).
        // Build per-shard linked lists by iterating blocks in reverse so the
        // lowest-offset block ends up as the shard head.

        // Reset all shard heads
        for s in 0..self.num_shards {
            self.shard_heads[s].store(usize::MAX, Ordering::Relaxed);
        }

        // SAFETY: `base_ptr` is valid for `total_blocks * block_size` bytes (checked in constructor),
        // and each block is large enough to hold a `usize` (enforced by minimum block_size).
        unsafe {
            for i in (0..self.total_blocks).rev() {
                let shard = i % self.num_shards;
                let offset = i * self.block_size;
                let block_ptr = self.base_ptr.as_ptr().add(offset) as *mut usize;
                let current_head = self.shard_heads[shard].load(Ordering::Relaxed);
                *block_ptr = current_head;
                self.shard_heads[shard].store(offset, Ordering::Relaxed);
            }
        }
    }

    fn is_valid_block_offset(&self, offset: usize) -> bool {
        offset.is_multiple_of(self.block_size) && offset < self.total_size
    }

    /// Select a shard for the current thread.
    fn thread_shard(&self) -> usize {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        (hasher.finish() as usize) % self.num_shards
    }

    /// Try to allocate from a specific shard. Returns None if shard is empty.
    fn try_allocate_from_shard(&self, shard: usize) -> Option<NonNull<u8>> {
        loop {
            let head_offset = self.shard_heads[shard].load(Ordering::Acquire);

            if head_offset == usize::MAX {
                return None;
            }

            // SAFETY: `head_offset` is a valid block offset within our memory region,
            // and the block contains a valid `usize` written by `initialize_free_list` or `deallocate`.
            let next_offset = unsafe {
                let head_ptr = self.base_ptr.as_ptr().add(head_offset) as *const usize;
                *head_ptr
            };

            match self.shard_heads[shard].compare_exchange_weak(
                head_offset,
                next_offset,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.allocated_count.fetch_add(1, Ordering::Relaxed);
                    // SAFETY: `head_offset` was validated as a block-aligned offset within the region.
                    let ptr = NonNull::new(unsafe { self.base_ptr.as_ptr().add(head_offset) });
                    return ptr;
                }
                Err(_) => {
                    std::hint::spin_loop();
                    continue;
                }
            }
        }
    }
}

impl Allocator for PoolAllocator {
    fn allocate(&self, size: usize, align: usize) -> Result<NonNull<u8>> {
        if size > self.block_size {
            return Err(RenoirError::insufficient_space(size, self.block_size));
        }

        if align > std::mem::align_of::<usize>() && !self.block_size.is_multiple_of(align) {
            return Err(RenoirError::alignment(0, align));
        }

        let home = self.thread_shard();

        // Try home shard first
        if let Some(ptr) = self.try_allocate_from_shard(home) {
            return Ok(ptr);
        }

        // Steal from other shards round-robin
        for i in 1..self.num_shards {
            let shard = (home + i) % self.num_shards;
            if let Some(ptr) = self.try_allocate_from_shard(shard) {
                return Ok(ptr);
            }
        }

        Err(RenoirError::insufficient_space(size, 0))
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

        // Return to the calling thread's shard to improve locality
        let shard = self.thread_shard();

        loop {
            let current_head = self.shard_heads[shard].load(Ordering::Acquire);

            // SAFETY: `ptr` is a valid block returned by `allocate`, block-aligned and within
            // our region, and is large enough to store a `usize`.
            unsafe {
                let block_ptr = ptr.as_ptr() as *mut usize;
                *block_ptr = current_head;
            }

            match self.shard_heads[shard].compare_exchange_weak(
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

        ptr_addr >= base_addr
            && ptr_addr < end_addr
            && self.is_valid_block_offset(ptr_addr - base_addr)
    }

    fn reset(&self) -> Result<()> {
        self.initialize_free_list();
        self.allocated_count.store(0, Ordering::Release);
        Ok(())
    }
}

unsafe impl Send for PoolAllocator {}
unsafe impl Sync for PoolAllocator {}
