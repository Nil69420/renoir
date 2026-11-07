//! Core topic management system for ROS2-style messaging

use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    sync::{atomic::Ordering, Arc},
};

use parking_lot::RwLock;

use crate::{
    error::{RenoirError, Result},
    memory::{BackingType, RegionConfig, SharedMemoryManager},
    shared_pools::BufferPoolRegistry,
    topic::{TopicConfig, TopicPattern, TopicStats},
};

use super::{
    handles::{Publisher, Subscriber},
    instance::TopicInstance,
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
            stats: TopicManagerStats::default(),
        })
    }

    /// Create or get an existing topic
    pub fn create_topic(&self, config: TopicConfig) -> Result<TopicId> {
        let topic_id = self.calculate_topic_id(&config.name);

        {
            let topics = self.topics.read();
            if let Some(existing) = topics.get(&config.name) {
                // A second registration with a meaningfully different config
                // must not be silently ignored: the first registration is
                // authoritative, and callers relying on a later call's options
                // taking effect would get silent misbehavior
                Self::validate_config_match(&existing.config, &config)?;
                return Ok(topic_id);
            }
        }

        // Create memory region for this topic with unique name.
        // Topics always use FileBacked regions so the region is attachable by
        // a second process; `enable_notifications` only controls eventfd
        // notification, which has zero coupling to the backing type.
        let region_name = format!("topic_{}_{}", config.name, topic_id);
        let region_config = RegionConfig {
            name: region_name.clone(),
            size: self.calculate_topic_memory_size(&config),
            backing_type: BackingType::FileBacked,
            // Topic names can contain '/' (e.g. "/sensors/imu"); the default
            // path derives from the region name, so sanitize it or the file
            // would land in a non-existent nested directory
            file_path: Some(crate::memory::region_path(&region_name.replace('/', "_"))),
            create: true,
            permissions: 0o600,
        };
        let memory_region = self.memory_manager.create_region(region_config)?;
        let topic_instance = Arc::new(TopicInstance::new(
            topic_id,
            config,
            memory_region,
            region_name,
            self.buffer_registry.clone(),
        )?);

        // Store topic
        {
            let mut topics = self.topics.write();
            let mut topic_ids = self.topic_ids.write();

            topics.insert(topic_instance.config.name.clone(), topic_instance.clone());
            topic_ids.insert(topic_id, topic_instance.config.name.clone());
        }

        self.stats.topics_created.fetch_add(1, Ordering::Relaxed);

        Ok(topic_id)
    }

    /// Create a publisher for a topic
    pub fn create_publisher(&self, topic_name: &str) -> Result<Publisher> {
        let topics = self.topics.read();
        let topic = topics
            .get(topic_name)
            .ok_or_else(|| RenoirError::invalid_parameter("topic_name", "Topic not found"))?
            .clone();

        let handle = topic.add_publisher()?;

        Ok(Publisher::new(handle, topic.topic_id, topic))
    }

    /// Create a subscriber for a topic
    pub fn create_subscriber(&self, topic_name: &str) -> Result<Subscriber> {
        let topics = self.topics.read();
        let topic = topics
            .get(topic_name)
            .ok_or_else(|| RenoirError::invalid_parameter("topic_name", "Topic not found"))?
            .clone();

        let handle = topic.add_subscriber()?;

        Ok(Subscriber::new(handle, topic.topic_id, topic))
    }

    /// Remove a topic (only if no active publishers/subscribers)
    pub fn remove_topic(&self, topic_name: &str) -> Result<()> {
        let topic = {
            let topics = self.topics.read();
            topics.get(topic_name).cloned()
        };

        if let Some(topic) = topic {
            if topic.stats.active_publishers.load(Ordering::Relaxed) > 0
                || topic.stats.active_subscribers.load(Ordering::Relaxed) > 0
            {
                return Err(RenoirError::invalid_parameter(
                    "topic_name",
                    "Cannot remove topic with active publishers/subscribers",
                ));
            }

            let mut topics = self.topics.write();
            let mut topic_ids = self.topic_ids.write();

            topics.remove(topic_name);
            topic_ids.remove(&topic.topic_id);

            // Clean up the memory region
            if let Ok(region) = self.memory_manager.get_region(&topic.region_name) {
                region.mark_session_clean();
            }
            if let Err(e) = self.memory_manager.remove_region(&topic.region_name) {
                // Log error but don't fail the removal
                log::warn!(
                    "Failed to remove memory region {}: {:?}",
                    topic.region_name,
                    e
                );
            }

            self.stats.topics_removed.fetch_add(1, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Mark every topic region as cleanly closed. C and Python callers
    /// release topics through FFI destroy calls rather than Rust Drop, so
    /// the CLEAN marker is written here on those paths, not only in Drop.
    pub fn mark_all_regions_clean(&self) {
        let topics = self.topics.read();
        for topic in topics.values() {
            if let Ok(region) = self.memory_manager.get_region(&topic.region_name) {
                region.mark_session_clean();
            }
        }
    }

    /// List all topics
    pub fn list_topics(&self) -> Vec<(String, TopicId, TopicConfig)> {
        let topics = self.topics.read();
        topics
            .iter()
            .map(|(name, topic)| (name.clone(), topic.topic_id, topic.config.clone()))
            .collect()
    }

    /// Get topic statistics
    pub fn topic_stats(&self, topic_name: &str) -> Result<Arc<TopicStats>> {
        let topics = self.topics.read();
        let topic = topics
            .get(topic_name)
            .ok_or_else(|| RenoirError::invalid_parameter("topic_name", "Topic not found"))?;

        Ok(topic.stats.clone())
    }

    /// Get global manager statistics
    pub fn manager_stats(&self) -> &TopicManagerStats {
        &self.stats
    }

    /// Check if a topic exists
    pub fn has_topic(&self, topic_name: &str) -> bool {
        let topics = self.topics.read();
        topics.contains_key(topic_name)
    }

    /// Get topic name by ID
    pub fn get_topic_name(&self, topic_id: TopicId) -> Option<String> {
        let topic_ids = self.topic_ids.read();
        topic_ids.get(&topic_id).cloned()
    }

    /// Get topic ID by name
    pub fn get_topic_id(&self, topic_name: &str) -> Option<TopicId> {
        let topics = self.topics.read();
        topics.get(topic_name).map(|topic| topic.topic_id)
    }

    /// Get topic count
    pub fn topic_count(&self) -> usize {
        let topics = self.topics.read();
        topics.len()
    }

    /// Get buffer registry (crate-internal use only)
    pub(crate) fn buffer_registry(&self) -> &Arc<BufferPoolRegistry> {
        &self.buffer_registry
    }

    fn calculate_topic_id(&self, name: &str) -> TopicId {
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        hasher.finish() as TopicId
    }

    /// Validate that a second registration of an existing topic name does not
    /// contradict the first registration's authoritative configuration.
    fn validate_config_match(existing: &TopicConfig, incoming: &TopicConfig) -> Result<()> {
        if incoming.pattern != existing.pattern {
            return Err(RenoirError::invalid_parameter(
                "pattern",
                format!(
                    "Topic '{}' already registered with pattern {:?}, cannot re-register with {:?}",
                    existing.name, existing.pattern, incoming.pattern
                ),
            ));
        }

        if incoming.max_payload_size != existing.max_payload_size {
            return Err(RenoirError::invalid_parameter(
                "max_payload_size",
                format!(
                    "Topic '{}' already registered with max_payload_size {}, cannot re-register with {}",
                    existing.name, existing.max_payload_size, incoming.max_payload_size
                ),
            ));
        }

        if incoming.use_shared_pool != existing.use_shared_pool
            || incoming.shared_pool_threshold != existing.shared_pool_threshold
        {
            return Err(RenoirError::invalid_parameter(
                "use_shared_pool",
                format!(
                    "Topic '{}' already registered with use_shared_pool {}/threshold {}, cannot re-register with {}/{}",
                    existing.name,
                    existing.use_shared_pool,
                    existing.shared_pool_threshold,
                    incoming.use_shared_pool,
                    incoming.shared_pool_threshold
                ),
            ));
        }

        Ok(())
    }

    fn calculate_topic_memory_size(&self, config: &TopicConfig) -> usize {
        // Ring control block + message buffer (ring_capacity is in KiB)
        let ring_size = match config.pattern {
            TopicPattern::SPSC => {
                crate::topic_rings::SPSCTopicRing::required_memory_size(config.ring_capacity * 1024)
            }
            TopicPattern::SPMC => crate::topic_rings::BroadcastTopicRing::required_memory_size(
                config.ring_capacity * 1024,
                config.max_payload_size,
            ),
            TopicPattern::MPSC | TopicPattern::MPMC => {
                crate::topic_rings::MPMCTopicRing::required_memory_size(config.ring_capacity * 1024)
            }
        };

        // Add buffer pool space if using shared pools
        let pool_size = if config.use_shared_pool {
            config.max_payload_size * 64 // Space for 64 large messages
        } else {
            0
        };

        (ring_size + pool_size).max(1024 * 1024) // At least 1MB
    }
}
