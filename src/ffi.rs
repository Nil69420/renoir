//! C Foreign Function Interface (FFI) for C++/ROS2 integration

use std::{
    ffi::{CStr, CString, c_char, c_void},
    ptr::null_mut,
    sync::Arc,
    collections::HashMap,
};

use crate::{
    SharedMemoryManager,
    allocator::Allocator,
    buffer::{Buffer, BufferPool, BufferPoolConfig},
    memory::{RegionConfig, BackingType},
    error::{RenoirError, Result},
};

/// Opaque handle types for C API
pub type RenoirManagerHandle = *mut c_void;
pub type RenoirRegionHandle = *mut c_void;
pub type RenoirBufferPoolHandle = *mut c_void;
pub type RenoirBufferHandle = *mut c_void;
pub type RenoirAllocatorHandle = *mut c_void;
pub type RenoirRingBufferHandle = *mut c_void;
pub type RenoirProducerHandle = *mut c_void;
pub type RenoirConsumerHandle = *mut c_void;

/// Error codes for C API
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenoirErrorCode {
    Success = 0,
    InvalidParameter = 1,
    OutOfMemory = 2,
    RegionNotFound = 3,
    RegionExists = 4,
    InsufficientSpace = 5,
    BufferFull = 6,
    BufferEmpty = 7,
    IoError = 8,
    SerializationError = 9,
    ConcurrencyError = 10,
    AlignmentError = 11,
    VersionMismatch = 12,
    PlatformError = 13,
    UnknownError = 99,
}

impl From<RenoirError> for RenoirErrorCode {
    fn from(error: RenoirError) -> Self {
        match error {
            RenoirError::InvalidParameter { .. } => RenoirErrorCode::InvalidParameter,
            RenoirError::Memory { .. } => RenoirErrorCode::OutOfMemory,
            RenoirError::RegionNotFound { .. } => RenoirErrorCode::RegionNotFound,
            RenoirError::RegionExists { .. } => RenoirErrorCode::RegionExists,
            RenoirError::InsufficientSpace { .. } => RenoirErrorCode::InsufficientSpace,
            RenoirError::BufferFull { .. } => RenoirErrorCode::BufferFull,
            RenoirError::BufferEmpty { .. } => RenoirErrorCode::BufferEmpty,
            RenoirError::Io { .. } => RenoirErrorCode::IoError,
            RenoirError::Serialization { .. } => RenoirErrorCode::SerializationError,
            RenoirError::Concurrency { .. } => RenoirErrorCode::ConcurrencyError,
            RenoirError::Alignment { .. } => RenoirErrorCode::AlignmentError,
            RenoirError::VersionMismatch { .. } => RenoirErrorCode::VersionMismatch,
            RenoirError::Platform { .. } => RenoirErrorCode::PlatformError,
        }
    }
}

/// Configuration structure for creating regions (C-compatible)
#[repr(C)]
pub struct RenoirRegionConfig {
    pub name: *const c_char,
    pub size: usize,
    pub backing_type: u32, // 0 = FileBacked, 1 = MemFd
    pub file_path: *const c_char,
    pub create: bool,
    pub permissions: u32,
}

/// Buffer pool configuration (C-compatible)
#[repr(C)]
pub struct RenoirBufferPoolConfig {
    pub name: *const c_char,
    pub buffer_size: usize,
    pub initial_count: usize,
    pub max_count: usize,
    pub alignment: usize,
    pub pre_allocate: bool,
    pub allocation_timeout_ms: u64, // 0 = no timeout
}

/// Buffer information structure
#[repr(C)]
pub struct RenoirBufferInfo {
    pub data: *mut u8,
    pub size: usize,
    pub capacity: usize,
    pub sequence: u64,
}

/// Statistics structures
#[repr(C)]
pub struct RenoirAllocatorStats {
    pub total_size: usize,
    pub used_size: usize,
    pub available_size: usize,
}

#[repr(C)]
pub struct RenoirBufferPoolStats {
    pub total_allocated: usize,
    pub currently_in_use: usize,
    pub peak_usage: usize,
    pub total_allocations: u64,
    pub total_deallocations: u64,
    pub allocation_failures: u64,
    pub success_rate: f64,
    pub utilization: f64,
}

// Global handle management
lazy_static::lazy_static! {
    static ref HANDLE_REGISTRY: std::sync::Mutex<HandleRegistry> = 
        std::sync::Mutex::new(HandleRegistry::new());
}

struct HandleRegistry {
    managers: HashMap<usize, Arc<SharedMemoryManager>>,
    regions: HashMap<usize, Arc<crate::memory::SharedMemoryRegion>>,
    buffer_pools: HashMap<usize, Arc<BufferPool>>,
    buffers: HashMap<usize, Box<Buffer>>,
    #[allow(dead_code)]
    allocators: HashMap<usize, Arc<dyn Allocator>>,
    #[allow(dead_code)]
    ring_buffers: HashMap<usize, Box<dyn std::any::Any + Send + Sync>>,
    next_id: usize,
}

impl HandleRegistry {
    fn new() -> Self {
        Self {
            managers: HashMap::new(),
            regions: HashMap::new(),
            buffer_pools: HashMap::new(),
            buffers: HashMap::new(),
            allocators: HashMap::new(),
            ring_buffers: HashMap::new(),
            next_id: 1,
        }
    }

    fn store_manager(&mut self, manager: Arc<SharedMemoryManager>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.managers.insert(id, manager);
        id
    }

    fn get_manager(&self, id: usize) -> Option<Arc<SharedMemoryManager>> {
        self.managers.get(&id).cloned()
    }

    fn remove_manager(&mut self, id: usize) -> Option<Arc<SharedMemoryManager>> {
        self.managers.remove(&id)
    }

    // Similar methods for other types...
    fn store_region(&mut self, region: Arc<crate::memory::SharedMemoryRegion>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.regions.insert(id, region);
        id
    }

    fn get_region(&self, id: usize) -> Option<Arc<crate::memory::SharedMemoryRegion>> {
        self.regions.get(&id).cloned()
    }

    fn store_buffer_pool(&mut self, pool: Arc<BufferPool>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.buffer_pools.insert(id, pool);
        id
    }

    fn get_buffer_pool(&self, id: usize) -> Option<Arc<BufferPool>> {
        self.buffer_pools.get(&id).cloned()
    }
}

// Helper functions
unsafe fn c_str_to_string(ptr: *const c_char) -> Result<String> {
    if ptr.is_null() {
        return Err(RenoirError::invalid_parameter("string", "Null pointer"));
    }
    
    CStr::from_ptr(ptr)
        .to_str()
        .map(|s| s.to_string())
        .map_err(|_| RenoirError::invalid_parameter("string", "Invalid UTF-8"))
}

fn string_to_c_str(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c_string) => c_string.into_raw(),
        Err(_) => null_mut(),
    }
}

// C API Functions

/// Initialize the Renoir library
#[no_mangle]
pub extern "C" fn renoir_init() -> RenoirErrorCode {
    env_logger::init();
    RenoirErrorCode::Success
}

/// Create a new shared memory manager
#[no_mangle]
pub extern "C" fn renoir_manager_create() -> RenoirManagerHandle {
    let manager = Arc::new(SharedMemoryManager::new());
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let id = registry.store_manager(manager);
    id as RenoirManagerHandle
}

/// Create a shared memory manager with control region
#[no_mangle]
pub extern "C" fn renoir_manager_create_with_control(
    config: *const RenoirRegionConfig
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
    let name = match unsafe { c_str_to_string(name) } {
        Ok(s) => s,
        Err(e) => return e.into(),
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

/// Create a buffer pool
#[no_mangle]
pub extern "C" fn renoir_buffer_pool_create(
    region: RenoirRegionHandle,
    config: *const RenoirBufferPoolConfig,
    pool_handle: *mut RenoirBufferPoolHandle,
) -> RenoirErrorCode {
    if region.is_null() || config.is_null() || pool_handle.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let region_id = region as usize;
    let config = unsafe { &*config };

    let registry = HANDLE_REGISTRY.lock().unwrap();
    let region = match registry.get_region(region_id) {
        Some(rgn) => rgn,
        None => return RenoirErrorCode::InvalidParameter,
    };
    drop(registry);

    let rust_config = match convert_buffer_pool_config(config) {
        Ok(cfg) => cfg,
        Err(e) => return e.into(),
    };

    match BufferPool::new(rust_config, region) {
        Ok(pool) => {
            let mut registry = HANDLE_REGISTRY.lock().unwrap();
            let id = registry.store_buffer_pool(Arc::new(pool));
            unsafe {
                *pool_handle = id as RenoirBufferPoolHandle;
            }
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

/// Get a buffer from a pool
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
        (*info).data = buffer.as_mut_ptr();
        (*info).size = buffer.size();
        (*info).capacity = buffer.capacity();
        (*info).sequence = buffer.sequence();
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

    let rust_stats = pool.stats();
    
    unsafe {
        (*stats).total_allocated = rust_stats.total_allocated;
        (*stats).currently_in_use = rust_stats.currently_in_use;
        (*stats).peak_usage = rust_stats.peak_usage;
        (*stats).total_allocations = rust_stats.total_allocations;
        (*stats).total_deallocations = rust_stats.total_deallocations;
        (*stats).allocation_failures = rust_stats.allocation_failures;
        (*stats).success_rate = rust_stats.success_rate();
        (*stats).utilization = rust_stats.utilization();
    }

    RenoirErrorCode::Success
}

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
        Some(b) => b,
        None => return RenoirErrorCode::InvalidParameter,
    };

    if let Err(err) = buffer.resize(new_size) {
        return err.into();
    }

    RenoirErrorCode::Success
}

#[no_mangle]
pub extern "C" fn renoir_buffer_set_sequence(
    buffer: RenoirBufferHandle,
    sequence: u64,
) -> RenoirErrorCode {
    if buffer.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let buffer_id = buffer as usize;
    
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    let buffer = match registry.buffers.get_mut(&buffer_id) {
        Some(b) => b,
        None => return RenoirErrorCode::InvalidParameter,
    };

    buffer.set_sequence(sequence);

    RenoirErrorCode::Success
}

#[no_mangle]
pub extern "C" fn renoir_buffer_pool_destroy(
    pool: RenoirBufferPoolHandle,
) -> RenoirErrorCode {
    if pool.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }

    let pool_id = pool as usize;
    
    let mut registry = HANDLE_REGISTRY.lock().unwrap();
    if registry.buffer_pools.remove(&pool_id).is_none() {
        return RenoirErrorCode::InvalidParameter;
    }

    RenoirErrorCode::Success
}

/// Free a C string allocated by Renoir
#[no_mangle]
pub extern "C" fn renoir_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}

// Helper conversion functions
fn convert_region_config(config: &RenoirRegionConfig) -> Result<RegionConfig> {
    let name = unsafe { c_str_to_string(config.name)? };
    
    let backing_type = match config.backing_type {
        0 => BackingType::FileBacked,
        #[cfg(target_os = "linux")]
        1 => BackingType::MemFd,
        _ => return Err(RenoirError::invalid_parameter("backing_type", "Invalid backing type")),
    };

    let file_path = if config.file_path.is_null() {
        None
    } else {
        Some(std::path::PathBuf::from(unsafe { c_str_to_string(config.file_path)? }))
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

fn convert_buffer_pool_config(config: &RenoirBufferPoolConfig) -> Result<BufferPoolConfig> {
    let name = unsafe { c_str_to_string(config.name)? };
    
    let allocation_timeout = if config.allocation_timeout_ms == 0 {
        None
    } else {
        Some(std::time::Duration::from_millis(config.allocation_timeout_ms))
    };

    Ok(BufferPoolConfig {
        name,
        buffer_size: config.buffer_size,
        initial_count: config.initial_count,
        max_count: config.max_count,
        alignment: config.alignment,
        pre_allocate: config.pre_allocate,
        allocation_timeout,
    })
}

// Version information
#[no_mangle]
pub extern "C" fn renoir_version_major() -> u32 {
    crate::VERSION_MAJOR
}

#[no_mangle]
pub extern "C" fn renoir_version_minor() -> u32 {
    crate::VERSION_MINOR
}

#[no_mangle]
pub extern "C" fn renoir_version_patch() -> u32 {
    crate::VERSION_PATCH
}

#[no_mangle]
pub extern "C" fn renoir_version_string() -> *mut c_char {
    string_to_c_str(crate::VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_error_code_conversion() {
        let error = RenoirError::region_not_found("test");
        let code: RenoirErrorCode = error.into();
        assert_eq!(code, RenoirErrorCode::RegionNotFound);
    }

    #[test]
    fn test_c_string_conversion() {
        let test_str = "test_string";
        let c_str = CString::new(test_str).unwrap();
        let converted = unsafe { c_str_to_string(c_str.as_ptr()).unwrap() };
        assert_eq!(converted, test_str);
    }
}