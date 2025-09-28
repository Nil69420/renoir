//! FFI functions for buffer management

use std::ffi::c_char;

use crate::buffers::{BufferPool, BufferPoolConfig};

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