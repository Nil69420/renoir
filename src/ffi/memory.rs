#![allow(clippy::not_unsafe_ptr_arg_deref)]
//! FFI functions for memory management

use std::{
    ffi::{c_char, CString},
    ptr::null_mut,
    sync::Arc,
};

use crate::{
    error::RenoirError,
    memory::{BackingType, RegionConfig, SharedMemoryManager},
};

use super::{
    types::{RenoirErrorCode, RenoirManagerHandle, RenoirRegionConfig, RenoirRegionHandle},
    utils::{c_str_to_string, string_to_c_str, HANDLE_REGISTRY},
};

/// Convert C region config to Rust config
fn convert_region_config(config: &RenoirRegionConfig) -> Result<RegionConfig, RenoirError> {
    let name = c_str_to_string(config.name).map_err(|_| RenoirError::InvalidParameter {
        parameter: "name".to_string(),
        message: "Invalid string".to_string(),
    })?;

    let backing_type = match config.backing_type {
        0 => BackingType::FileBacked,
        1 => BackingType::MemFd,
        _ => {
            return Err(RenoirError::InvalidParameter {
                parameter: "backing_type".to_string(),
                message: "Must be 0 (FileBacked) or 1 (MemFd)".to_string(),
            })
        }
    };

    let file_path = if !config.file_path.is_null() {
        Some(
            c_str_to_string(config.file_path)
                .map_err(|_| RenoirError::InvalidParameter {
                    parameter: "file_path".to_string(),
                    message: "Invalid string".to_string(),
                })?
                .into(),
        )
    } else {
        None
    };

    Ok(RegionConfig {
        name,
        size: config.size,
        backing_type,
        file_path,
        create: config.create,
        permissions: config.permissions,
    })
}

/// Create a shared memory manager with control region
#[no_mangle]
pub extern "C" fn renoir_manager_create_with_control(
    config: *const RenoirRegionConfig,
) -> RenoirManagerHandle {
    if config.is_null() {
        return null_mut();
    }

    let config = unsafe { &*config };

    let rust_config = match convert_region_config(config) {
        Ok(cfg) => cfg,
        Err(_) => return null_mut(),
    };

    match SharedMemoryManager::with_control_region(rust_config) {
        Ok(manager) => {
            let mut registry = HANDLE_REGISTRY.lock();
            let id = registry.store_manager(Arc::new(manager));
            id as RenoirManagerHandle
        }
        Err(_) => null_mut(),
    }
}

/// Create a shared memory manager without control region
#[no_mangle]
pub extern "C" fn renoir_manager_create() -> RenoirManagerHandle {
    let manager = SharedMemoryManager::new();
    let mut registry = HANDLE_REGISTRY.lock();
    let id = registry.store_manager(Arc::new(manager));
    id as RenoirManagerHandle
}

/// Destroy a shared memory manager
#[no_mangle]
pub extern "C" fn renoir_manager_destroy(handle: RenoirManagerHandle) -> RenoirErrorCode {
    if handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let id = handle as usize;
    let mut registry = HANDLE_REGISTRY.lock();

    if registry.remove_manager(id).is_some() {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Create a shared memory region
#[no_mangle]
pub extern "C" fn renoir_region_create(
    manager: RenoirManagerHandle,
    config: *const RenoirRegionConfig,
    region_handle: *mut RenoirRegionHandle,
) -> RenoirErrorCode {
    if manager.is_null() || config.is_null() || region_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let config = unsafe { &*config };

    let registry = HANDLE_REGISTRY.lock();
    let manager = match registry.get_manager(manager_id) {
        Some(mgr) => mgr,
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    let rust_config = match convert_region_config(config) {
        Ok(cfg) => cfg,
        Err(e) => return e.into(),
    };

    match manager.create_region(rust_config) {
        Ok(region) => {
            let mut registry = HANDLE_REGISTRY.lock();
            let id = registry.store_region(region);
            unsafe {
                *region_handle = id as RenoirRegionHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Get an existing shared memory region
#[no_mangle]
pub extern "C" fn renoir_region_get(
    manager: RenoirManagerHandle,
    name: *const c_char,
    region_handle: *mut RenoirRegionHandle,
) -> RenoirErrorCode {
    if manager.is_null() || name.is_null() || region_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let name = match c_str_to_string(name) {
        Ok(s) => s,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    let registry = HANDLE_REGISTRY.lock();
    let manager = match registry.get_manager(manager_id) {
        Some(mgr) => mgr,
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    match manager.get_region(&name) {
        Ok(region) => {
            let mut registry = HANDLE_REGISTRY.lock();
            let id = registry.store_region(region);
            unsafe {
                *region_handle = id as RenoirRegionHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Get region size
#[no_mangle]
pub extern "C" fn renoir_region_size(handle: RenoirRegionHandle) -> usize {
    if handle.is_null() {
        return 0;
    }

    let id = handle as usize;
    let registry = HANDLE_REGISTRY.lock();

    registry
        .get_region(id)
        .map(|region| region.size())
        .unwrap_or(0)
}

/// Get region name
#[no_mangle]
pub extern "C" fn renoir_region_name(handle: RenoirRegionHandle) -> *mut c_char {
    if handle.is_null() {
        return null_mut();
    }

    let id = handle as usize;
    let registry = HANDLE_REGISTRY.lock();

    registry
        .get_region(id)
        .map(|region| string_to_c_str(region.name().to_string()))
        .unwrap_or(null_mut())
}

/// Open existing region by name
#[no_mangle]
pub extern "C" fn renoir_region_open(
    manager: RenoirManagerHandle,
    name: *const c_char,
    region_handle: *mut RenoirRegionHandle,
) -> RenoirErrorCode {
    if manager.is_null() || name.is_null() || region_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let name_str = match c_str_to_string(name) {
        Ok(s) => s,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    match mgr.get_region(&name_str) {
        Ok(region) => {
            drop(registry);
            let mut registry = HANDLE_REGISTRY.lock();
            let id = registry.store_region(region);
            unsafe {
                *region_handle = id as RenoirRegionHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Close region handle (decreases reference count)
#[no_mangle]
pub extern "C" fn renoir_region_close(handle: RenoirRegionHandle) -> RenoirErrorCode {
    if handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let id = handle as usize;
    let mut registry = HANDLE_REGISTRY.lock();

    if let Some(region) = registry.regions.remove(&id) {
        region.mark_session_clean();
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::InvalidParameter
    }
}

/// Destroy region (admin operation - removes from all processes)
#[no_mangle]
pub extern "C" fn renoir_region_destroy(
    manager: RenoirManagerHandle,
    name: *const c_char,
) -> RenoirErrorCode {
    if manager.is_null() || name.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let name_str = match c_str_to_string(name) {
        Ok(s) => s,
        Err(_) => return RenoirErrorCode::InvalidParameter,
    };

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    if let Ok(region) = mgr.get_region(&name_str) {
        region.mark_session_clean();
    }

    match mgr.remove_region(&name_str) {
        Ok(_) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

/// Get region statistics
#[no_mangle]
pub extern "C" fn renoir_region_stats(
    handle: RenoirRegionHandle,
    stats: *mut super::types::RenoirRegionStats,
) -> RenoirErrorCode {
    if handle.is_null() || stats.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let id = handle as usize;
    let registry = HANDLE_REGISTRY.lock();

    let region = match registry.get_region(id) {
        Some(r) => r,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let _region_stats = region.memory_stats();

    let used_size = region.allocated_bytes();
    unsafe {
        (*stats).total_size = region.size();
        // used_size is the sum of bytes currently allocated from this region
        // by its buffer pools (real accounting, not a placeholder)
        (*stats).used_size = used_size;
        (*stats).free_size = region.size().saturating_sub(used_size);
        // Fragmentation ratio: not available for the allocator types used
        // with regions today (fixed-block pool / bump) — neither maintains
        // free-run contiguity bookkeeping, so there is no honest number to
        // report. -1.0 is the documented "not available" sentinel.
        (*stats).fragmentation_ratio = -1.0;
        (*stats).topic_count = 0;
        (*stats).allocation_count = 0;
    }

    RenoirErrorCode::Success
}

/// List all regions managed by the manager
#[no_mangle]
pub extern "C" fn renoir_manager_list_regions(
    manager: RenoirManagerHandle,
    region_names: *mut *mut *mut c_char,
    count: *mut usize,
) -> RenoirErrorCode {
    if manager.is_null() || region_names.is_null() || count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let regions = mgr.list_regions();
    let region_count = regions.len();

    if region_count == 0 {
        unsafe {
            *region_names = std::ptr::null_mut();
            *count = 0;
        }
        return RenoirErrorCode::Success;
    }

    // Allocate array of C strings
    let names_array = unsafe {
        libc::malloc(region_count * std::mem::size_of::<*mut c_char>()) as *mut *mut c_char
    };

    if names_array.is_null() {
        return RenoirErrorCode::OutOfMemory;
    }

    // Convert each region name to C string
    for (i, name) in regions.iter().enumerate() {
        let c_name = string_to_c_str(name.clone());
        unsafe {
            *names_array.add(i) = c_name;
        }
    }

    unsafe {
        *region_names = names_array;
        *count = region_count;
    }

    RenoirErrorCode::Success
}

/// Free region names list allocated by renoir_manager_list_regions
#[no_mangle]
pub extern "C" fn renoir_free_region_names(region_names: *mut *mut c_char, count: usize) {
    if region_names.is_null() {
        return;
    }

    // Free individual strings
    for i in 0..count {
        unsafe {
            let name_ptr = *region_names.add(i);
            if !name_ptr.is_null() {
                let _ = CString::from_raw(name_ptr);
            }
        }
    }

    // Free the array
    unsafe {
        libc::free(region_names as *mut std::ffi::c_void);
    }
}

/// Get total memory usage across all regions
#[no_mangle]
pub extern "C" fn renoir_manager_total_memory_usage(
    manager: RenoirManagerHandle,
    total_bytes: *mut usize,
) -> RenoirErrorCode {
    if manager.is_null() || total_bytes.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *total_bytes = mgr.total_memory_usage();
    }

    RenoirErrorCode::Success
}

/// Get number of regions managed
#[no_mangle]
pub extern "C" fn renoir_manager_region_count(
    manager: RenoirManagerHandle,
    count: *mut usize,
) -> RenoirErrorCode {
    if manager.is_null() || count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    unsafe {
        *count = mgr.region_count();
    }

    RenoirErrorCode::Success
}

/// Flush all regions to persistent storage
#[no_mangle]
pub extern "C" fn renoir_manager_flush_all(manager: RenoirManagerHandle) -> RenoirErrorCode {
    if manager.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    match mgr.flush_all() {
        Ok(_) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

/// Clear all regions from manager
#[no_mangle]
pub extern "C" fn renoir_manager_clear_all(manager: RenoirManagerHandle) -> RenoirErrorCode {
    if manager.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    match mgr.clear() {
        Ok(_) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}
