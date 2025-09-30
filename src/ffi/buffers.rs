//! FFI functions for buffer management

use std::ffi::c_char;
use std::any::Any;
use std::sync::Arc;

use crate::buffers::{BufferPool, BufferPoolConfig};
use crate::ringbuf::{RingBuffer, SequencedRingBuffer, Producer, Consumer, ClaimGuard};
use crate::metadata_modules::{RegionMetadata, RegionRegistryEntry, ControlStats, ControlRegion};

use super::{
    types::*,
    utils::*,
};
use super::types::RenoirBufferPoolStats;

/// Convert C buffer pool config to Rust config
fn convert_buffer_pool_config(config: &RenoirBufferPoolConfig) -> Result<BufferPoolConfig, RenoirErrorCode> {
    let name = c_str_to_string(config.name)
        .map_err(|_| RenoirErrorCode::InvalidParameter)?;

    Ok(BufferPoolConfig {
        name,
        buffer_size: config.buffer_size,
        initial_count: config.initial_count,
        max_count: config.max_count,
        alignment: config.alignment,
        pre_allocate: config.pre_allocate,
        allocation_timeout: if config.allocation_timeout_ms == 0 {
            None
        } else {
            Some(std::time::Duration::from_millis(config.allocation_timeout_ms))
        },
    })
}

/// Create a buffer pool
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_create(
    manager_handle: RenoirManagerHandle,
    region_name: *const c_char,
    config: *const RenoirBufferPoolConfig,
    pool_handle: *mut RenoirBufferPoolHandle,
) -> RenoirErrorCode {
    if config.is_null() || pool_handle.is_null() || region_name.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let config = unsafe { &*config };
    let rust_config = match convert_buffer_pool_config(config) {
        Ok(cfg) => cfg,
        Err(e) => return e,
    };

    // Get the manager and memory region
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let manager = match registry.get_manager(manager_handle as usize) {
        Some(mgr) => mgr,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let region_name_str = match c_str_to_string(region_name) {
        Ok(name) => name,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    let region = match manager.get_region(&region_name_str) {
        Ok(region) => region,
        Err(_) => return RenoirErrorCode::RegionNotFound,
    };

    match BufferPool::new(rust_config, region) {
        Ok(pool) => {
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let id = registry.store_buffer_pool(std::sync::Arc::new(pool));
            unsafe {
                *pool_handle = id as RenoirBufferPoolHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Get a buffer from the pool
#[no_mangle]
pub extern "C" fn renoir_buffer_get(
    pool: RenoirBufferPoolHandle,
    buffer_handle: *mut RenoirBufferHandle,
) -> RenoirErrorCode {
    if pool.is_null() || buffer_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    match pool.get_buffer() {
        Ok(buffer) => {
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let id = registry.next_id;
            registry.next_id += 1;
            registry.buffers.insert(id, Box::new(buffer));
            unsafe {
                *buffer_handle = id as RenoirBufferHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Get buffer information
#[no_mangle]
pub extern "C" fn renoir_buffer_info(
    buffer: RenoirBufferHandle,
    info: *mut RenoirBufferInfo,
) -> RenoirErrorCode {
    if buffer.is_null() || info.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let buffer_id = buffer as usize;
    
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer = match registry.buffers.get(&buffer_id) {
        Some(buf) => buf,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        (*info).size = buffer.size();
        (*info).capacity = buffer.capacity();
        (*info).alignment = buffer.alignment();
        (*info).pool_name = std::ptr::null(); // Would need pool reference
    }

    RenoirErrorCode::Success
}

/// Get buffer sequence number
#[no_mangle]
pub extern "C" fn renoir_buffer_sequence(
    buffer: RenoirBufferHandle,
    sequence: *mut u64,
) -> RenoirErrorCode {
    if buffer.is_null() || sequence.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let buffer_id = buffer as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer = match registry.buffers.get(&buffer_id) {
        Some(buf) => buf,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *sequence = buffer.sequence();
    }

    RenoirErrorCode::Success
}

/// Resize a buffer (if supported by allocator)
#[no_mangle]
pub extern "C" fn renoir_buffer_resize(
    buffer: RenoirBufferHandle,
    new_size: usize,
) -> RenoirErrorCode {
    if buffer.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let buffer_id = buffer as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer = match registry.buffers.get_mut(&buffer_id) {
        Some(buf) => buf,
        None => return RenoirErrorCode::InvalidParameter,
    };

    match buffer.resize(new_size) {
        Ok(()) => RenoirErrorCode::Success,
        Err(_) => RenoirErrorCode::OutOfMemory,
    }
}

/// Clear buffer contents (sets size to 0)
#[no_mangle]
pub extern "C" fn renoir_buffer_clear(buffer: RenoirBufferHandle) -> RenoirErrorCode {
    if buffer.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let buffer_id = buffer as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer = match registry.buffers.get_mut(&buffer_id) {
        Some(buf) => buf,
        None => return RenoirErrorCode::InvalidParameter,
    };

    buffer.clear();
    RenoirErrorCode::Success
}

/// Get read-only access to buffer data
#[no_mangle]
pub extern "C" fn renoir_buffer_data(
    buffer: RenoirBufferHandle,
    data_ptr: *mut *const u8,
    data_len: *mut usize,
) -> RenoirErrorCode {
    if buffer.is_null() || data_ptr.is_null() || data_len.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let buffer_id = buffer as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer = match registry.buffers.get(&buffer_id) {
        Some(buf) => buf,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let slice = buffer.as_slice();
    unsafe {
        *data_ptr = slice.as_ptr();
        *data_len = slice.len();
    }

    RenoirErrorCode::Success
}

/// Get mutable access to buffer data
#[no_mangle]
pub extern "C" fn renoir_buffer_data_mut(
    buffer: RenoirBufferHandle,
    data_ptr: *mut *mut u8,
    data_len: *mut usize,
) -> RenoirErrorCode {
    if buffer.is_null() || data_ptr.is_null() || data_len.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let buffer_id = buffer as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer = match registry.buffers.get_mut(&buffer_id) {
        Some(buf) => buf,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let slice = buffer.as_mut_slice();
    unsafe {
        *data_ptr = slice.as_mut_ptr();
        *data_len = slice.len();
    }

    RenoirErrorCode::Success
}

/// Return a buffer to its pool
#[no_mangle]
pub extern "C" fn renoir_buffer_return(
    pool: RenoirBufferPoolHandle,
    buffer: RenoirBufferHandle,
) -> RenoirErrorCode {
    if pool.is_null() || buffer.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    let buffer_id = buffer as usize;

    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer = match registry.buffers.remove(&buffer_id) {
        Some(buf) => *buf,
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    match pool.return_buffer(buffer) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

/// Get buffer pool statistics
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_stats(
    pool: RenoirBufferPoolHandle,
    stats: *mut RenoirBufferPoolStats,
) -> RenoirErrorCode {
    if pool.is_null() || stats.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let pool_stats = pool.stats();
    
    unsafe {
        (*stats).total_allocated = pool_stats.total_allocated;
        (*stats).currently_in_use = pool_stats.currently_in_use;
        (*stats).peak_usage = pool_stats.peak_usage;
        (*stats).total_allocations = pool_stats.total_allocations;
        (*stats).total_deallocations = pool_stats.total_deallocations;
        (*stats).allocation_failures = pool_stats.allocation_failures;
        (*stats).success_rate = pool_stats.success_rate();
        (*stats).utilization = pool_stats.utilization();
        (*stats).failure_rate = pool_stats.failure_rate();
        (*stats).is_healthy = pool_stats.is_healthy();
    }

    RenoirErrorCode::Success
}

/// Get computed buffer pool statistics (rates and health indicators only)
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_computed_stats(
    pool: RenoirBufferPoolHandle,
    success_rate: *mut f64,
    utilization: *mut f64,
    failure_rate: *mut f64,
    is_healthy: *mut bool,
) -> RenoirErrorCode {
    if pool.is_null() || success_rate.is_null() || utilization.is_null() 
        || failure_rate.is_null() || is_healthy.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let pool_stats = pool.stats();
    
    unsafe {
        *success_rate = pool_stats.success_rate();
        *utilization = pool_stats.utilization();
        *failure_rate = pool_stats.failure_rate();
        *is_healthy = pool_stats.is_healthy();
    }

    RenoirErrorCode::Success
}

/// Get the number of available buffers in the pool
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_available_count(
    pool: RenoirBufferPoolHandle,
    count: *mut usize,
) -> RenoirErrorCode {
    if pool.is_null() || count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *count = pool.available_count();
    }

    RenoirErrorCode::Success
}

/// Get the number of allocated buffers in the pool
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_allocated_count(
    pool: RenoirBufferPoolHandle,
    count: *mut usize,
) -> RenoirErrorCode {
    if pool.is_null() || count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *count = pool.allocated_count();
    }

    RenoirErrorCode::Success
}

/// Check if the buffer pool can expand
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_can_expand(
    pool: RenoirBufferPoolHandle,
    can_expand: *mut bool,
) -> RenoirErrorCode {
    if pool.is_null() || can_expand.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *can_expand = pool.can_expand();
    }

    RenoirErrorCode::Success
}

/// Shrink the buffer pool to the target number of available buffers
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_shrink(
    pool: RenoirBufferPoolHandle,
    target_available: usize,
    shrunk_count: *mut usize,
) -> RenoirErrorCode {
    if pool.is_null() || shrunk_count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *shrunk_count = pool.shrink(target_available);
    }

    RenoirErrorCode::Success
}

/// Get buffer pool configuration
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_config(
    pool: RenoirBufferPoolHandle,
    config: *mut RenoirBufferPoolConfig,
) -> RenoirErrorCode {
    if pool.is_null() || config.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    let pool = match registry.get_buffer_pool(pool_id) {
        Some(p) => p,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let pool_config = pool.config();
    
    unsafe {
        // Note: The name pointer will be invalidated after this function returns
        // In practice, the C caller should copy the string if needed
        (*config).name = pool_config.name.as_ptr() as *const c_char;
        (*config).buffer_size = pool_config.buffer_size;
        (*config).initial_count = pool_config.initial_count;
        (*config).max_count = pool_config.max_count;
        (*config).alignment = pool_config.alignment;
        (*config).pre_allocate = pool_config.pre_allocate;
        (*config).allocation_timeout_ms = match pool_config.allocation_timeout {
            Some(duration) => duration.as_millis() as u64,
            None => 0,
        };
    }

    RenoirErrorCode::Success
}

/// Destroy a buffer pool
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_destroy(pool: RenoirBufferPoolHandle) -> RenoirErrorCode {
    if pool.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    
    if registry.buffer_pools.remove(&pool_id).is_some() {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

//
// Ring Buffer FFI Operations
//

/// Create a byte ring buffer with given capacity (must be power of 2)
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_create(
    capacity: usize,
    ring_buffer_handle: *mut RenoirRingBufferHandle,
) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    match RingBuffer::<u8>::new(capacity) {
        Ok(ring_buffer) => {
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let handle = registry.next_id;
            registry.next_id += 1;
            registry.ring_buffers.insert(handle, Box::new(ring_buffer));
            unsafe { *ring_buffer_handle = handle as RenoirRingBufferHandle };
            RenoirErrorCode::Success
        }
        Err(_) => RenoirErrorCode::InvalidParameter,
    }
}

/// Get ring buffer statistics
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_stats(
    ring_buffer_handle: RenoirRingBufferHandle,
    stats: *mut RenoirRingStats,
) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() || stats.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = ring_buffer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            unsafe {
                (*stats).capacity = ring_buffer.capacity();
                (*stats).size = ring_buffer.len();
                (*stats).writes = 0; // Basic ring buffer doesn't track writes/reads
                (*stats).reads = 0;
                (*stats).overwrites = 0;
            }
            return RenoirErrorCode::Success;
        }
    }
    
    RenoirErrorCode::InvalidParameter
}

/// Check if ring buffer is empty
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_is_empty(
    ring_buffer_handle: RenoirRingBufferHandle,
    is_empty: *mut bool,
) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() || is_empty.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = ring_buffer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            unsafe { *is_empty = ring_buffer.is_empty() };
            return RenoirErrorCode::Success;
        }
    }
    
    RenoirErrorCode::InvalidParameter
}

/// Check if ring buffer is full
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_is_full(
    ring_buffer_handle: RenoirRingBufferHandle,
    is_full: *mut bool,
) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() || is_full.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = ring_buffer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            unsafe { *is_full = ring_buffer.is_full() };
            return RenoirErrorCode::Success;
        }
    }
    
    RenoirErrorCode::InvalidParameter
}

/// Get available space in ring buffer
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_available_space(
    ring_buffer_handle: RenoirRingBufferHandle,
    available_space: *mut usize,
) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() || available_space.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = ring_buffer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            unsafe { *available_space = ring_buffer.available_space() };
            return RenoirErrorCode::Success;
        }
    }
    
    RenoirErrorCode::InvalidParameter
}

/// Reset ring buffer (clear all data)
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_reset(ring_buffer_handle: RenoirRingBufferHandle) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = ring_buffer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            ring_buffer.reset();
            return RenoirErrorCode::Success;
        }
    }
    
    RenoirErrorCode::InvalidParameter
}

/// Get a producer for the ring buffer
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_producer(
    ring_buffer_handle: RenoirRingBufferHandle,
    producer_handle: *mut RenoirProducerHandle,
) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() || producer_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    // Note: For simplicity, we'll return the same handle as the ring buffer
    // In practice, you might want separate producer/consumer handle management
    unsafe { *producer_handle = ring_buffer_handle };
    RenoirErrorCode::Success
}

/// Get a consumer for the ring buffer
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_consumer(
    ring_buffer_handle: RenoirRingBufferHandle,
    consumer_handle: *mut RenoirConsumerHandle,
) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() || consumer_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    // Note: For simplicity, we'll return the same handle as the ring buffer
    // In practice, you might want separate producer/consumer handle management
    unsafe { *consumer_handle = ring_buffer_handle };
    RenoirErrorCode::Success
}

/// Producer: Try to push a byte to the ring buffer (non-blocking)
#[no_mangle]
pub extern "C" fn renoir_ring_producer_try_push(
    producer_handle: RenoirProducerHandle,
    data: u8,
) -> RenoirErrorCode {
    if producer_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = producer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            let producer = ring_buffer.producer();
            match producer.try_push(data) {
                Ok(()) => RenoirErrorCode::Success,
                Err(_) => RenoirErrorCode::BufferFull,
            }
        } else {
            RenoirErrorCode::InvalidParameter
        }
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Producer: Push a byte to the ring buffer (blocking)
#[no_mangle]
pub extern "C" fn renoir_ring_producer_push(
    producer_handle: RenoirProducerHandle,
    data: u8,
) -> RenoirErrorCode {
    if producer_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = producer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            let producer = ring_buffer.producer();
            match producer.push(data) {
                Ok(()) => RenoirErrorCode::Success,
                Err(_) => RenoirErrorCode::BufferFull,
            }
        } else {
            RenoirErrorCode::InvalidParameter
        }
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Consumer: Try to pop a byte from the ring buffer (non-blocking)
#[no_mangle]
pub extern "C" fn renoir_ring_consumer_try_pop(
    consumer_handle: RenoirConsumerHandle,
    data: *mut u8,
) -> RenoirErrorCode {
    if consumer_handle.is_null() || data.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = consumer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            let consumer = ring_buffer.consumer();
            match consumer.try_pop() {
                Ok(value) => {
                    unsafe { *data = value };
                    RenoirErrorCode::Success
                }
                Err(_) => RenoirErrorCode::BufferEmpty,
            }
        } else {
            RenoirErrorCode::InvalidParameter
        }
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Consumer: Pop a byte from the ring buffer (blocking)
#[no_mangle]
pub extern "C" fn renoir_ring_consumer_pop(
    consumer_handle: RenoirConsumerHandle,
    data: *mut u8,
) -> RenoirErrorCode {
    if consumer_handle.is_null() || data.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = consumer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            let consumer = ring_buffer.consumer();
            match consumer.pop() {
                Ok(value) => {
                    unsafe { *data = value };
                    RenoirErrorCode::Success
                }
                Err(_) => RenoirErrorCode::BufferEmpty,
            }
        } else {
            RenoirErrorCode::InvalidParameter
        }
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Consumer: Peek at the next byte without consuming it
#[no_mangle]
pub extern "C" fn renoir_ring_consumer_peek(
    consumer_handle: RenoirConsumerHandle,
    data: *mut u8,
) -> RenoirErrorCode {
    if consumer_handle.is_null() || data.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = consumer_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    if let Some(any_ring_buffer) = registry.ring_buffers.get(&handle) {
        if let Some(ring_buffer) = any_ring_buffer.downcast_ref::<RingBuffer<u8>>() {
            let consumer = ring_buffer.consumer();
            match consumer.peek() {
                Ok(value) => {
                    unsafe { *data = *value };
                    RenoirErrorCode::Success
                }
                Err(_) => RenoirErrorCode::BufferEmpty,
            }
        } else {
            RenoirErrorCode::InvalidParameter
        }
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Destroy a ring buffer
#[no_mangle]
pub extern "C" fn renoir_ring_buffer_destroy(ring_buffer_handle: RenoirRingBufferHandle) -> RenoirErrorCode {
    if ring_buffer_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let handle = ring_buffer_handle as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    
    if registry.ring_buffers.remove(&handle).is_some() {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

//
// Metadata and Control Region Operations
//

/// Create a control region for metadata management
#[no_mangle]
pub extern "C" fn renoir_control_region_create(
    config: *const RenoirRegionConfig,
    control_region_handle: *mut RenoirControlRegionHandle,
) -> RenoirErrorCode {
    if config.is_null() || control_region_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let config_ref = unsafe { &*config };
    
    // Convert C config to Rust config
    let name = match c_str_to_string(config_ref.name) {
        Ok(name) => name,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    let mut region_config = crate::memory::RegionConfig::new(name, config_ref.size);
    
    match config_ref.backing_type {
        0 => region_config.backing_type = crate::memory::BackingType::FileBacked,
        #[cfg(target_os = "linux")]
        1 => region_config.backing_type = crate::memory::BackingType::MemFd,
        _ => return RenoirErrorCode::InvalidParameter,
    }

    match ControlRegion::new(region_config) {
        Ok(control_region) => {
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let handle = registry.next_id;
            registry.next_id += 1;
            registry.control_regions.insert(handle, Arc::new(control_region));
            unsafe { *control_region_handle = handle as RenoirControlRegionHandle };
            RenoirErrorCode::Success
        }
        Err(_) => RenoirErrorCode::OutOfMemory,
    }
}

/// Register buffer pool metadata in control region  
#[no_mangle]
pub extern "C" fn renoir_control_region_register_buffer_pool(
    control_handle: RenoirControlRegionHandle,
    pool_handle: RenoirBufferPoolHandle,
) -> RenoirErrorCode {
    if control_handle.is_null() || pool_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let control_id = control_handle as usize;
    let pool_id = pool_handle as usize;
    
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    let control_region = match registry.control_regions.get(&control_id) {
        Some(control) => control.clone(),
        None => return RenoirErrorCode::InvalidParameter,
    };
    
    let buffer_pool = match registry.buffer_pools.get(&pool_id) {
        Some(pool) => pool.clone(),
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    let pool_config = buffer_pool.config();
    let metadata = RegionMetadata::new(
        format!("buffer_pool_{}", pool_config.name),
        pool_config.buffer_size * pool_config.max_count,
        crate::memory::BackingType::FileBacked
    );

    match control_region.register_region(&metadata) {
        Ok(()) => RenoirErrorCode::Success,
        Err(_) => RenoirErrorCode::InvalidParameter,
    }
}

/// Get control region statistics
#[no_mangle]
pub extern "C" fn renoir_control_region_stats(
    control_handle: RenoirControlRegionHandle,
    stats: *mut RenoirControlStats,
) -> RenoirErrorCode {
    if control_handle.is_null() || stats.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let control_id = control_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    let control_region = match registry.control_regions.get(&control_id) {
        Some(control) => control.clone(),
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    let control_stats = control_region.get_stats();
    
    unsafe {
        (*stats).sequence_number = control_stats.global_sequence;
        (*stats).total_regions = control_stats.total_regions;
        (*stats).active_regions = control_stats.total_regions; // ControlStats doesn't separate active/stale
        (*stats).stale_regions = 0; // We don't have this info directly
    }

    RenoirErrorCode::Success
}

/// List all registered regions in control region
#[no_mangle]
pub extern "C" fn renoir_control_region_list_regions(
    control_handle: RenoirControlRegionHandle,
    entries: *mut *mut RenoirRegionRegistryEntry,
    count: *mut usize,
) -> RenoirErrorCode {
    if control_handle.is_null() || entries.is_null() || count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let control_id = control_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    let control_region = match registry.control_regions.get(&control_id) {
        Some(control) => control.clone(),
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    let region_entries = control_region.list_regions();
    
    if region_entries.is_empty() {
        unsafe {
            *entries = std::ptr::null_mut();
            *count = 0;
        }
        return RenoirErrorCode::Success;
    }

    // Allocate C array for entries
    let layout = match std::alloc::Layout::array::<RenoirRegionRegistryEntry>(region_entries.len()) {
        Ok(layout) => layout,
        Err(_) => return RenoirErrorCode::OutOfMemory,
    };
    
    let c_entries = unsafe { std::alloc::alloc(layout) as *mut RenoirRegionRegistryEntry };
    if c_entries.is_null() {
        return RenoirErrorCode::OutOfMemory;
    }

    for (i, entry) in region_entries.iter().enumerate() {
        let c_name = match std::ffi::CString::new(entry.metadata.name.clone()) {
            Ok(name) => name,
            Err(_) => return RenoirErrorCode::InvalidParameter,
        };
        
        let backing_type = match entry.metadata.backing_type {
            crate::memory::BackingType::FileBacked => 0,
            #[cfg(target_os = "linux")]
            crate::memory::BackingType::MemFd => 1,
        };

        unsafe {
            let c_entry = &mut *c_entries.add(i);
            c_entry.name = c_name.into_raw();
            c_entry.size = entry.metadata.size;
            c_entry.backing_type = backing_type;
            c_entry.version = entry.metadata.version;
            c_entry.ref_count = entry.ref_count;
            c_entry.creator_pid = entry.creator_pid;
        }
    }

    unsafe {
        *entries = c_entries;
        *count = region_entries.len();
    }

    RenoirErrorCode::Success
}

/// Free region registry entries array allocated by renoir_control_region_list_regions
#[no_mangle]
pub extern "C" fn renoir_free_region_entries(
    entries: *mut RenoirRegionRegistryEntry,
    count: usize,
) -> RenoirErrorCode {
    if entries.is_null() {
        return RenoirErrorCode::Success; // Nothing to free
    }

    unsafe {
        // Free individual name strings
        for i in 0..count {
            let entry = &mut *entries.add(i);
            if !entry.name.is_null() {
                let _ = std::ffi::CString::from_raw(entry.name as *mut std::ffi::c_char);
            }
        }
        
        // Free the array
        let layout = match std::alloc::Layout::array::<RenoirRegionRegistryEntry>(count) {
            Ok(layout) => layout,
            Err(_) => return RenoirErrorCode::InvalidParameter,
        };
        std::alloc::dealloc(entries as *mut u8, layout);
    }

    RenoirErrorCode::Success
}

/// Cleanup stale entries in control region
#[no_mangle]
pub extern "C" fn renoir_control_region_cleanup_stale(
    control_handle: RenoirControlRegionHandle,
    max_age_seconds: u64,
    cleaned_count: *mut usize,
) -> RenoirErrorCode {
    if control_handle.is_null() || cleaned_count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let control_id = control_handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();
    
    let control_region = match registry.control_regions.get(&control_id) {
        Some(control) => control.clone(),
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    match control_region.cleanup_stale_entries(max_age_seconds) {
        Ok(count) => {
            unsafe { *cleaned_count = count };
            RenoirErrorCode::Success
        }
        Err(_) => RenoirErrorCode::InvalidParameter,
    }
}

/// Destroy a control region
#[no_mangle]
pub extern "C" fn renoir_control_region_destroy(control_handle: RenoirControlRegionHandle) -> RenoirErrorCode {
    if control_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let control_id = control_handle as usize;
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    
    if registry.control_regions.remove(&control_id).is_some() {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}