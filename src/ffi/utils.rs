//! FFI utilities and handle management

use std::{
    ffi::{CStr, CString, c_char},
    collections::HashMap,
    sync::Arc,
};

use crate::{
    SharedMemoryManager,
    allocators::Allocator,
    buffers::{Buffer, BufferPool},
    topic_manager_modules::{TopicManager, Publisher, Subscriber},
};



// Global handle management
lazy_static::lazy_static! {
    pub static ref HANDLE_REGISTRY: std::sync::Mutex<HandleRegistry> = 
        std::sync::Mutex::new(HandleRegistry::new());
}

pub struct HandleRegistry {
    pub managers: HashMap<usize, Arc<SharedMemoryManager>>,
    pub regions: HashMap<usize, Arc<crate::memory::SharedMemoryRegion>>,
    pub buffer_pools: HashMap<usize, Arc<BufferPool>>,
    pub buffers: HashMap<usize, Box<Buffer>>,
    #[allow(dead_code)]
    pub allocators: HashMap<usize, Arc<dyn Allocator>>,
    #[allow(dead_code)]
    pub ring_buffers: HashMap<usize, Box<dyn std::any::Any + Send + Sync>>,
    #[allow(dead_code)]
    pub control_regions: HashMap<usize, Arc<crate::metadata_modules::ControlRegion>>,
    pub topic_managers: HashMap<usize, Arc<TopicManager>>,
    pub publishers: HashMap<usize, Publisher>,
    pub subscribers: HashMap<usize, Subscriber>,
    pub reserved_buffers: HashMap<usize, Vec<u8>>,
    pub received_messages: HashMap<usize, crate::topic::Message>,
    pub next_id: usize,
}

impl HandleRegistry {
    pub fn new() -> Self {
        Self {
            managers: HashMap::new(),
            regions: HashMap::new(),
            buffer_pools: HashMap::new(),
            buffers: HashMap::new(),
            allocators: HashMap::new(),
            ring_buffers: HashMap::new(),
            control_regions: HashMap::new(),
            topic_managers: HashMap::new(),
            publishers: HashMap::new(),
            subscribers: HashMap::new(),
            reserved_buffers: HashMap::new(),
            received_messages: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn store_manager(&mut self, manager: Arc<SharedMemoryManager>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.managers.insert(id, manager);
        id
    }

    pub fn get_manager(&self, id: usize) -> Option<Arc<SharedMemoryManager>> {
        self.managers.get(&id).cloned()
    }

    pub fn remove_manager(&mut self, id: usize) -> Option<Arc<SharedMemoryManager>> {
        self.managers.remove(&id)
    }

    pub fn store_region(&mut self, region: Arc<crate::memory::SharedMemoryRegion>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.regions.insert(id, region);
        id
    }

    pub fn get_region(&self, id: usize) -> Option<Arc<crate::memory::SharedMemoryRegion>> {
        self.regions.get(&id).cloned()
    }

    pub fn store_buffer_pool(&mut self, pool: Arc<BufferPool>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.buffer_pools.insert(id, pool);
        id
    }

    pub fn get_buffer_pool(&self, id: usize) -> Option<Arc<BufferPool>> {
        self.buffer_pools.get(&id).cloned()
    }
}

/// Convert C string to Rust String
pub fn c_str_to_string(c_str: *const c_char) -> Result<String, Box<dyn std::error::Error>> {
    if c_str.is_null() {
        return Ok(String::new());
    }
    
    unsafe {
        CStr::from_ptr(c_str)
            .to_str()
            .map(|s| s.to_owned())
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
    }
}

/// Convert Rust String to C string (caller must free with renoir_free_string)
pub fn string_to_c_str(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c_string) => c_string.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a C string allocated by this library
#[no_mangle]
pub extern "C" fn renoir_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}