//! FFI functions for memory management

use std::{
    ffi::c_char,
    ptr::null_mut,
    sync::Arc,
};

use crate::{
    SharedMemoryManager,
    memory::{RegionConfig, BackingType},
    error::RenoirError,
};

use super::{
    types::{RenoirErrorCode, RenoirManagerHandle, RenoirRegionHandle, RenoirRegionConfig},
    utils::{HANDLE_REGISTRY, c_str_to_string, string_to_c_str},
};

/// Convert C region config to Rust config
fn convert_region_config(config: &RenoirRegionConfig) -> Result<RegionConfig, RenoirError> {
    let name = c_str_to_string(config.name)
        .map_err(|_| RenoirError::InvalidParameter {
            parameter: "name".to_string(),
            message: "Invalid string".to_string(),
        })?;

    let backing_type = match config.backing_type {
        0 => BackingType::FileBacked,
        1 => BackingType::MemFd,
        _ => return Err(RenoirError::InvalidParameter {
            parameter: "backing_type".to_string(),
            message: "Must be 0 (FileBacked) or 1 (MemFd)".to_string(),
        }),
    };

    let file_path = if !config.file_path.is_null() {
        Some(c_str_to_string(config.file_path)
            .map_err(|_| RenoirError::InvalidParameter {
                parameter: "file_path".to_string(),
                message: "Invalid string".to_string(),
            })?.into())
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
    
    registry.get_region(id)
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
    
    registry.get_region(id)
        .map(|region| string_to_c_str(region.name().to_string()))
        .unwrap_or(null_mut())
}