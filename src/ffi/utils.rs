//! FFI utilities and handle management

use std::{
    collections::HashMap,
    ffi::{c_char, CStr, CString},
    sync::Arc,
};

use crate::{
    allocators::Allocator,
    buffers::{Buffer, BufferPool},
    shared_pools::BufferPoolRegistry,
    topic_manager_modules::{Publisher, Subscriber, TopicManager},
    SharedMemoryManager,
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
    pub buffer_registries: HashMap<usize, Arc<BufferPoolRegistry>>, // Maps topic_manager_id -> BufferPoolRegistry
    pub publishers: HashMap<usize, Arc<Publisher>>,
    pub subscribers: HashMap<usize, Arc<Subscriber>>,
    pub subscriber_to_manager: HashMap<usize, usize>, // Maps subscriber_id -> manager_id
    pub publisher_to_manager: HashMap<usize, usize>, // Maps publisher_id -> manager_id
    pub reserved_buffers: HashMap<usize, Vec<u8>>,
    pub received_messages: HashMap<usize, crate::topic::Message>,
    pub descriptor_buffers: HashMap<usize, Arc<Buffer>>, // Keeps descriptor buffers alive
    pub message_to_descriptor_buffer: HashMap<usize, usize>, // Maps message_id -> descriptor_buffer_id
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
            buffer_registries: HashMap::new(),
            publishers: HashMap::new(),
            subscribers: HashMap::new(),
            subscriber_to_manager: HashMap::new(),
            publisher_to_manager: HashMap::new(),
            reserved_buffers: HashMap::new(),
            received_messages: HashMap::new(),
            descriptor_buffers: HashMap::new(),
            message_to_descriptor_buffer: HashMap::new(),
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

    // Topic Manager methods
    pub fn store_topic_manager(&mut self, manager: Arc<TopicManager>, registry: Arc<BufferPoolRegistry>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.topic_managers.insert(id, manager);
        self.buffer_registries.insert(id, registry);
        id
    }

    pub fn get_topic_manager(&self, id: usize) -> Option<Arc<TopicManager>> {
        self.topic_managers.get(&id).cloned()
    }

    pub fn get_buffer_registry(&self, manager_id: usize) -> Option<Arc<BufferPoolRegistry>> {
        self.buffer_registries.get(&manager_id).cloned()
    }

    pub fn remove_topic_manager(&mut self, id: usize) -> bool {
        self.buffer_registries.remove(&id);
        self.topic_managers.remove(&id).is_some()
    }

    // Publisher methods
    pub fn store_publisher(&mut self, publisher: Arc<Publisher>, manager_id: usize) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.publishers.insert(id, publisher);
        self.publisher_to_manager.insert(id, manager_id);
        id
    }

    pub fn get_publisher(&self, id: usize) -> Option<Arc<Publisher>> {
        self.publishers.get(&id).cloned()
    }

    pub fn remove_publisher(&mut self, id: usize) -> bool {
        self.publisher_to_manager.remove(&id);
        self.publishers.remove(&id).is_some()
    }

    // Subscriber methods
    pub fn store_subscriber(&mut self, subscriber: Arc<Subscriber>, manager_id: usize) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.subscribers.insert(id, subscriber);
        self.subscriber_to_manager.insert(id, manager_id);
        id
    }

    pub fn get_subscriber(&self, id: usize) -> Option<Arc<Subscriber>> {
        self.subscribers.get(&id).cloned()
    }

    pub fn get_subscriber_manager_id(&self, id: usize) -> Option<usize> {
        self.subscriber_to_manager.get(&id).copied()
    }

    pub fn remove_subscriber(&mut self, id: usize) -> bool {
        self.subscriber_to_manager.remove(&id);
        self.subscribers.remove(&id).is_some()
    }

    // Message methods
    pub fn store_message(&mut self, message: Box<crate::topic::Message>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.received_messages.insert(id, *message);
        id
    }

    pub fn get_message(&self, id: usize) -> Option<&crate::topic::Message> {
        self.received_messages.get(&id)
    }

    pub fn remove_message(&mut self, id: usize) -> bool {
        // Clean up associated descriptor buffer if any
        if let Some(buf_id) = self.message_to_descriptor_buffer.remove(&id) {
            self.descriptor_buffers.remove(&buf_id);
        }
        self.received_messages.remove(&id).is_some()
    }

    pub fn associate_descriptor_buffer(&mut self, message_id: usize, buffer_id: usize) {
        self.message_to_descriptor_buffer.insert(message_id, buffer_id);
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
