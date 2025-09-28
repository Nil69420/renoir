//! Main structured memory region manager

use std::{
    mem::size_of,
    sync::Arc,
};

use crate::{
    error::{RenoirError, Result},
    memory::SharedMemoryRegion,
};

use super::{
    constants::*,
    headers::*,
};

/// Main structured memory region manager
pub struct StructuredMemoryRegion {
    region: Arc<SharedMemoryRegion>,
    global_header: *mut GlobalControlHeader,
    topic_directory: *mut TopicDirectory,
}

unsafe impl Send for StructuredMemoryRegion {}
unsafe impl Sync for StructuredMemoryRegion {}

impl StructuredMemoryRegion {
    /// Create or attach to a structured memory region
    pub fn create(region: Arc<SharedMemoryRegion>, initialize: bool) -> Result<Self> {
        let base_ptr: *mut u8 = unsafe { region.as_mut_ptr_unsafe() };
        let global_header = base_ptr as *mut GlobalControlHeader;
        
        if initialize {
            // Initialize the global header
            let header = GlobalControlHeader::new(region.size() as u64, MAX_TOPICS as u32);
            unsafe {
                std::ptr::write(global_header, header);
            }
            
            // Initialize the topic directory
            let topic_directory = unsafe { 
                base_ptr.add(size_of::<GlobalControlHeader>()) as *mut TopicDirectory 
            };
            unsafe {
                std::ptr::write(topic_directory, TopicDirectory::new());
            }
        } else {
            // Validate existing region
            unsafe {
                (*global_header).validate()?;
            }
        }
        
        let topic_directory = unsafe {
            base_ptr.add((*global_header).topic_directory_offset as usize) as *mut TopicDirectory
        };
        
        Ok(Self {
            region,
            global_header,
            topic_directory,
        })
    }

    /// Find a topic by name
    pub fn find_topic(&self, name: &str) -> Option<&TopicDirectoryEntry> {
        let name_bytes = name.as_bytes();
        if name_bytes.len() >= MAX_TOPIC_NAME_LEN {
            return None;
        }
        
        let directory = unsafe { &*self.topic_directory };
        
        for i in 0..directory.entry_count as usize {
            let entry = &directory.entries[i];
            if entry.valid == 0 {
                continue;
            }
            
            let entry_name_len = entry.name.iter()
                .position(|&x| x == 0)
                .unwrap_or(MAX_TOPIC_NAME_LEN);
            
            if entry_name_len == name_bytes.len() && 
               &entry.name[..entry_name_len] == name_bytes {
                return Some(entry);
            }
        }
        
        None
    }

    /// Create a new topic
    pub fn create_topic(&mut self, name: &str, ring_size: u32, schema_id: u32) -> Result<u32> {
        let name_bytes = name.as_bytes();
        if name_bytes.len() >= MAX_TOPIC_NAME_LEN {
            return Err(RenoirError::invalid_parameter("name", "Topic name too long"));
        }
        
        // Check if topic already exists
        if self.find_topic(name).is_some() {
            return Err(RenoirError::invalid_parameter("name", "Topic already exists"));
        }
        
        let global_header = unsafe { &mut *self.global_header };
        let topic_directory = unsafe { &mut *self.topic_directory };
        
        if topic_directory.entry_count >= MAX_TOPICS as u32 {
            return Err(RenoirError::invalid_parameter("topic_count", "Too many topics"));
        }
        
        // Generate new topic ID
        let topic_id = global_header.global_sequence + 1;
        global_header.global_sequence = topic_id;
        
        // Find free entry slot
        let entry_index = topic_directory.entry_count as usize;
        let entry = &mut topic_directory.entries[entry_index];
        
        // Initialize entry
        entry.name[..name_bytes.len()].copy_from_slice(name_bytes);
        entry.name[name_bytes.len()..].fill(0);
        entry.topic_id = topic_id as u32;
        entry.ring_size = ring_size;
        entry.schema_id = schema_id;
        entry.valid = 1;
        
        // Update directory metadata
        topic_directory.entry_count += 1;
        global_header.topic_count += 1;
        
        // Set validity bit
        let bit_index = entry_index;
        let word_index = bit_index / 64;
        let bit_offset = bit_index % 64;
        topic_directory.validity_bitmap[word_index] |= 1u64 << bit_offset;
        
        Ok(topic_id as u32)
    }

    /// Get global statistics
    pub fn get_global_stats(&self) -> GlobalStats {
        let header = unsafe { &*self.global_header };
        GlobalStats {
            magic: header.magic,
            version: header.version,
            creation_time: header.creation_time,
            owner_pid: header.owner_pid,
            region_size: header.region_size,
            max_topics: header.max_topics,
            topic_count: header.topic_count,
            global_sequence: header.global_sequence,
        }
    }

    /// Get topic control header for a topic
    pub fn get_topic_header(&self, topic_offset: u32) -> Result<*mut TopicControlHeader> {
        let base_ptr: *mut u8 = unsafe { self.region.as_mut_ptr_unsafe() };
        let topic_header = unsafe {
            base_ptr.add(topic_offset as usize) as *mut TopicControlHeader
        };
        Ok(topic_header)
    }
}

/// Global statistics structure
#[derive(Debug, Clone)]
pub struct GlobalStats {
    pub magic: u64,
    pub version: u32,
    pub creation_time: u64,
    pub owner_pid: u32,
    pub region_size: u64,
    pub max_topics: u32,
    pub topic_count: u32,
    pub global_sequence: u64,
}