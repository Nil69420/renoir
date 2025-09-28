//! Core topic management system for ROS2-style messaging

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    sync::{Arc, RwLock, atomic::{AtomicU32, Ordering}},
    hash::{Hash, Hasher},
};

use crate::{
    error::{RenoirError, Result},
    memory::{SharedMemoryManager, RegionConfig, BackingType},
    topic::{TopicConfig, TopicStats},
    shared_pools::BufferPoolRegistry,
};

use super::{
    instance::TopicInstance,
    handles::{Publisher, Subscriber},
    stats::TopicManagerStats,
};

/// Unique identifier for topics
pub type TopicId = u32;

/// Topic management system
#[derive(Debug)]
pub struct TopicManager {
    /// Map of topic name to topic instance
    topics: RwLock<HashMap<String, Arc<TopicInstance>>>,
    /// Map of topic ID to topic name for reverse lookup
    topic_ids: RwLock<HashMap<TopicId, String>>,
    /// Shared memory manager for creating regions
    memory_manager: Arc<SharedMemoryManager>,
    /// Buffer pool registry for large messages
    buffer_registry: Arc<BufferPoolRegistry>,
    /// Next available topic ID
    #[allow(dead_code)]
    next_topic_id: AtomicU32,
    /// Global statistics
    stats: TopicManagerStats,
}

impl TopicManager {
    /// Create a new topic manager
    pub fn new() -> Result<Self> {
        let memory_manager = Arc::new(SharedMemoryManager::new());
        
        // Create default shared memory region for buffer pools
        let default_config = RegionConfig {
            name: "renoir_topic_pools".to_string(),
            size: 256 * 1024 * 1024, // 256MB default
            backing_type: BackingType::MemFd,
            file_path: None,
            create: true,
            permissions: 0o600,
        };
        
        let default_region = memory_manager.create_region(default_config)?;
        let buffer_registry = Arc::new(BufferPoolRegistry::new(default_region));
        
        Ok(Self {
            topics: RwLock::new(HashMap::new()),
            topic_ids: RwLock::new(HashMap::new()),
            memory_manager,
            buffer_registry,
            next_topic_id: AtomicU32::new(1),
            stats: TopicManagerStats::default(),
        })
    }

    /// Create or get an existing topic
    pub fn create_topic(&self, config: TopicConfig) -> Result<TopicId> {
        let topic_id = self.calculate_topic_id(&config.name);
        
        {
            let topics = self.topics.read().unwrap();
            if topics.contains_key(&config.name) {
                return Ok(topic_id);
            }
        }

        // Create memory region for this topic with unique name
        let region_config = RegionConfig {
            name: format!("topic_{}_{}", config.name, topic_id),
            size: self.calculate_topic_memory_size(&config),
            backing_type: if config.enable_notifications { 
                BackingType::MemFd 
            } else { 
                BackingType::FileBacked 
            },
            file_path: None,
            create: true,
            permissions: 0o600,
        };
        
        let region_name = region_config.name.clone();
        let memory_region = self.memory_manager.create_region(region_config)?;
        let topic_instance = Arc::new(TopicInstance::new(
            topic_id, 
            config, 
            memory_region, 
            region_name, 
            self.buffer_registry.clone()
        )?);
        
        // Store topic
        {
            let mut topics = self.topics.write().unwrap();
            let mut topic_ids = self.topic_ids.write().unwrap();
            
            topics.insert(topic_instance.config.name.clone(), topic_instance.clone());
            topic_ids.insert(topic_id, topic_instance.config.name.clone());
        }
        
        self.stats.topics_created.fetch_add(1, Ordering::Relaxed);
        
        Ok(topic_id)
    }

    /// Create a publisher for a topic
    pub fn create_publisher(&self, topic_name: &str) -> Result<Publisher> {
        let topics = self.topics.read().unwrap();
        let topic = topics.get(topic_name)
            .ok_or_else(|| RenoirError::invalid_parameter("topic_name", "Topic not found"))?
            .clone();
            
        let handle = topic.add_publisher()?;
        
        Ok(Publisher::new(handle, topic.topic_id, topic))
    }

    /// Create a subscriber for a topic
    pub fn create_subscriber(&self, topic_name: &str) -> Result<Subscriber> {
        let topics = self.topics.read().unwrap();
        let topic = topics.get(topic_name)
            .ok_or_else(|| RenoirError::invalid_parameter("topic_name", "Topic not found"))?
            .clone();
            
        let handle = topic.add_subscriber()?;
        
        Ok(Subscriber::new(handle, topic.topic_id, topic))
    }

    /// Remove a topic (only if no active publishers/subscribers)
    pub fn remove_topic(&self, topic_name: &str) -> Result<()> {
        let topic = {
            let topics = self.topics.read().unwrap();
            topics.get(topic_name).cloned()
        };

        if let Some(topic) = topic {
            if topic.stats.active_publishers.load(Ordering::Relaxed) > 0 ||
               topic.stats.active_subscribers.load(Ordering::Relaxed) > 0 {
                return Err(RenoirError::invalid_parameter(
                    "topic_name", 
                    "Cannot remove topic with active publishers/subscribers"
                ));
            }

            let mut topics = self.topics.write().unwrap();
            let mut topic_ids = self.topic_ids.write().unwrap();
            
            topics.remove(topic_name);
            topic_ids.remove(&topic.topic_id);
            
            // Clean up the memory region
            if let Err(e) = self.memory_manager.remove_region(&topic.region_name) {
                // Log error but don't fail the removal
                eprintln!("Warning: Failed to remove memory region {}: {:?}", topic.region_name, e);
            }
            
            self.stats.topics_removed.fetch_add(1, Ordering::Relaxed);
        }

        Ok(())
    }

    /// List all topics
    pub fn list_topics(&self) -> Vec<(String, TopicId, TopicConfig)> {
        let topics = self.topics.read().unwrap();
        topics.iter()
            .map(|(name, topic)| (name.clone(), topic.topic_id, topic.config.clone()))
            .collect()
    }

    /// Get topic statistics
    pub fn topic_stats(&self, topic_name: &str) -> Result<Arc<TopicStats>> {
        let topics = self.topics.read().unwrap();
        let topic = topics.get(topic_name)
            .ok_or_else(|| RenoirError::invalid_parameter("topic_name", "Topic not found"))?;
            
        Ok(topic.stats.clone())
    }

    /// Get global manager statistics
    pub fn manager_stats(&self) -> &TopicManagerStats {
        &self.stats
    }

    /// Get topic instance by name (internal use)
    #[allow(dead_code)]
    pub(super) fn get_topic_instance(&self, topic_name: &str) -> Option<Arc<TopicInstance>> {
        let topics = self.topics.read().unwrap();
        topics.get(topic_name).cloned()
    }

    /// Check if a topic exists
    pub fn has_topic(&self, topic_name: &str) -> bool {
        let topics = self.topics.read().unwrap();
        topics.contains_key(topic_name)
    }

    /// Get topic name by ID
    pub fn get_topic_name(&self, topic_id: TopicId) -> Option<String> {
        let topic_ids = self.topic_ids.read().unwrap();
        topic_ids.get(&topic_id).cloned()
    }

    /// Get topic count
    pub fn topic_count(&self) -> usize {
        let topics = self.topics.read().unwrap();
        topics.len()
    }

    /// Get memory manager (internal use)
    #[allow(dead_code)]
    pub(super) fn memory_manager(&self) -> &Arc<SharedMemoryManager> {
        &self.memory_manager
    }

    /// Get buffer registry (internal use)  
    #[allow(dead_code)]
    pub(super) fn buffer_registry(&self) -> &Arc<BufferPoolRegistry> {
        &self.buffer_registry
    }

    fn calculate_topic_id(&self, name: &str) -> TopicId {
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        hasher.finish() as TopicId
    }

    fn calculate_topic_memory_size(&self, config: &TopicConfig) -> usize {
        // Ring buffer size + overhead
        let ring_size = config.ring_capacity * (config.max_payload_size + 128); // Header overhead
        
        // Add buffer pool space if using shared pools
        let pool_size = if config.use_shared_pool {
            config.max_payload_size * 64 // Space for 64 large messages
        } else {
            0
        };
        
        (ring_size + pool_size).max(1024 * 1024) // At least 1MB
    }
}

impl Default for TopicManager {
    fn default() -> Self {
        Self::new().expect("Failed to create default TopicManager")
    }
}