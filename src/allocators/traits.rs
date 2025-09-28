//! Allocator trait definition

use std::ptr::NonNull;
use crate::error::Result;

/// Trait for shared memory allocators
pub trait Allocator: Send + Sync + std::fmt::Debug {
    /// Allocate memory of the given size and alignment
    fn allocate(&self, size: usize, align: usize) -> Result<NonNull<u8>>;
    
    /// Deallocate previously allocated memory
    fn deallocate(&self, ptr: NonNull<u8>, size: usize) -> Result<()>;
    
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
    
    /// Get allocator alignment requirement
    fn alignment(&self) -> usize {
        std::mem::align_of::<usize>()
    }
    
    /// Check if allocator supports deallocation
    fn supports_deallocation(&self) -> bool {
        true
    }
    
    /// Get allocator type name for debugging
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Common allocator operations
pub trait AllocatorExt: Allocator {
    /// Allocate and zero-initialize memory
    fn allocate_zeroed(&self, size: usize, align: usize) -> Result<NonNull<u8>> {
        let ptr = self.allocate(size, align)?;
        unsafe {
            std::ptr::write_bytes(ptr.as_ptr(), 0, size);
        }
        Ok(ptr)
    }
    
    /// Allocate memory for a specific type
    fn allocate_for<T>(&self) -> Result<NonNull<T>> {
        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        let ptr = self.allocate(size, align)?;
        Ok(ptr.cast::<T>())
    }
    
    /// Deallocate memory for a specific type
    fn deallocate_for<T>(&self, ptr: NonNull<T>) -> Result<()> {
        let size = std::mem::size_of::<T>();
        self.deallocate(ptr.cast::<u8>(), size)
    }
    
    /// Check if allocator has enough space for allocation
    fn can_allocate(&self, size: usize, _align: usize) -> bool {
        self.available_size() >= size
    }
    
    /// Get utilization percentage (0.0 to 1.0)
    fn utilization(&self) -> f64 {
        if self.total_size() == 0 {
            return 0.0;
        }
        self.used_size() as f64 / self.total_size() as f64
    }
}

// Blanket implementation for all Allocators
impl<T: Allocator + ?Sized> AllocatorExt for T {}

/// Allocator capabilities flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocatorCapabilities {
    /// Supports arbitrary deallocation
    pub supports_deallocation: bool,
    /// Supports resetting/clearing all allocations
    pub supports_reset: bool,
    /// Thread-safe for concurrent access
    pub thread_safe: bool,
    /// Memory is zeroed on allocation
    pub zero_initialized: bool,
    /// Supports alignment requirements
    pub supports_alignment: bool,
}

impl Default for AllocatorCapabilities {
    fn default() -> Self {
        Self {
            supports_deallocation: true,
            supports_reset: false,
            thread_safe: true,
            zero_initialized: false,
            supports_alignment: true,
        }
    }
}