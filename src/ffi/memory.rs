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
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
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
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
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
    let mut registry = HANDLE_REGISTRY.lock().unwrap();

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

    let registry = HANDLE_REGISTRY.lock().unwrap();
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
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
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

    let registry = HANDLE_REGISTRY.lock().unwrap();
    let manager = match registry.get_manager(manager_id) {
        Some(mgr) => mgr,
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    match manager.get_region(&name) {
        Ok(region) => {
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
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
    let registry = HANDLE_REGISTRY.lock().unwrap();

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
    let registry = HANDLE_REGISTRY.lock().unwrap();

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
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    match mgr.get_region(&name_str) {
        Ok(region) => {
            drop(registry);
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
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
    let mut registry = HANDLE_REGISTRY.lock().unwrap();

    if registry.regions.remove(&id).is_some() {
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
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

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
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let region = match registry.get_region(id) {
        Some(r) => r,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let _region_stats = region.memory_stats();

    unsafe {
        (*stats).total_size = region.size();
        // Note: Fine-grained memory usage tracking would require:
        // 1. Instrumenting the allocator to track allocations/deallocations
        // 2. Maintaining allocation metadata in the region
        // 3. Periodic scanning for fragmentation analysis
        // These are complex features that would impact performance.
        // For now, we return basic size information.
        (*stats).used_size = 0;
        (*stats).free_size = region.size();
        (*stats).fragmentation_ratio = 0.0;
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
    let registry = HANDLE_REGISTRY.lock().unwrap();

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

/// Check if a region exists
#[no_mangle]
pub extern "C" fn renoir_manager_has_region(
    manager: RenoirManagerHandle,
    name: *const c_char,
) -> bool {
    if manager.is_null() || name.is_null() {
        return false;
    }

    let name_str = match c_str_to_string(name) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let manager_id = manager as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return false,
    };

    mgr.has_region(&name_str)
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
    let registry = HANDLE_REGISTRY.lock().unwrap();

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
    let registry = HANDLE_REGISTRY.lock().unwrap();

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
    let registry = HANDLE_REGISTRY.lock().unwrap();

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
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let mgr = match registry.get_manager(manager_id) {
        Some(m) => m,
        None => return RenoirErrorCode::InvalidParameter,
    };

    match mgr.clear() {
        Ok(_) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

/// Get region metadata
#[no_mangle]
pub extern "C" fn renoir_region_metadata(
    handle: RenoirRegionHandle,
    name: *mut *mut c_char,
    size: *mut usize,
    backing_type: *mut u32,
) -> RenoirErrorCode {
    if handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let id = handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let region = match registry.get_region(id) {
        Some(r) => r,
        None => return RenoirErrorCode::InvalidParameter,
    };

    let metadata = region.metadata();

    if !name.is_null() {
        unsafe {
            *name = string_to_c_str(metadata.name.clone());
        }
    }

    if !size.is_null() {
        unsafe {
            *size = metadata.size;
        }
    }

    if !backing_type.is_null() {
        unsafe {
            *backing_type = match metadata.backing_type {
                crate::memory::BackingType::FileBacked => 0,
                #[cfg(target_os = "linux")]
                crate::memory::BackingType::MemFd => 1,
            };
        }
    }

    RenoirErrorCode::Success
}

/// Create allocator for a region
#[no_mangle]
pub extern "C" fn renoir_region_create_bump_allocator(
    handle: RenoirRegionHandle,
    allocator_handle: *mut super::types::RenoirAllocatorHandle,
) -> RenoirErrorCode {
    if handle.is_null() || allocator_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let id = handle as usize;
    let registry = HANDLE_REGISTRY.lock().unwrap();

    let region = match registry.get_region(id) {
        Some(r) => r,
        None => return RenoirErrorCode::InvalidParameter,
    };

    // Get region memory as slice for allocator
    // SAFETY: We're creating an allocator that will manage exclusive access
    let memory_slice =
        unsafe { std::slice::from_raw_parts_mut(region.as_mut_ptr_unsafe::<u8>(), region.size()) };

    match crate::allocators::BumpAllocator::new(memory_slice) {
        Ok(allocator) => {
            drop(registry);
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let alloc_id = registry.next_id;
            registry.next_id += 1;
            registry
                .allocators
                .insert(alloc_id, std::sync::Arc::new(allocator));

            unsafe {
                *allocator_handle = alloc_id as super::types::RenoirAllocatorHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}
