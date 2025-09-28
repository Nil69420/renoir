//! Topic management system for ROS2-style messaging
//!
//! This module provides a complete topic management system that automatically
//! selects appropriate ring buffer types and shared buffer pools based on
//! topic configuration and usage patterns.

use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    sync::{Arc, RwLock, atomic::{AtomicUsize, AtomicU32, Ordering}},
    hash::{Hash, Hasher},
    time::{Duration, Instant},
    thread,
};

use crate::{
    error::{RenoirError, Result},
    memory::{SharedMemoryRegion, SharedMemoryManager, RegionConfig, BackingType},
    topic::{TopicConfig, TopicPattern, TopicStats, Message},
    topic_rings::{SPSCTopicRing, MPMCTopicRing},
    shared_pools::BufferPoolRegistry,
};

/// Unique identifier for topics
pub type TopicId = u32;

/// Handle for publishers
pub type PublisherHandle = usize;

/// Handle for subscribers  
pub type SubscriberHandle = usize;

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
            backing_type: if config.enable_notifications { BackingType::MemFd } else { BackingType::FileBacked },
            file_path: None,
            create: true,
            permissions: 0o600,
        };
        
        let region_name = region_config.name.clone();
        let memory_region = self.memory_manager.create_region(region_config)?;
        let topic_instance = Arc::new(TopicInstance::new(topic_id, config, memory_region, region_name, self.buffer_registry.clone())?);
        
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
        
        Ok(Publisher {
            handle,
            topic_id: topic.topic_id,
            topic: topic.clone(),
        })
    }

    /// Create a subscriber for a topic
    pub fn create_subscriber(&self, topic_name: &str) -> Result<Subscriber> {
        let topics = self.topics.read().unwrap();
        let topic = topics.get(topic_name)
            .ok_or_else(|| RenoirError::invalid_parameter("topic_name", "Topic not found"))?
            .clone();
            
        let handle = topic.add_subscriber()?;
        
        Ok(Subscriber {
            handle,
            topic_id: topic.topic_id,
            topic: topic.clone(),
            last_sequence: AtomicUsize::new(0),
        })
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

/// Individual topic instance with appropriate ring buffer and configuration
#[derive(Debug)]
pub struct TopicInstance {
    /// Topic identifier
    pub topic_id: TopicId,
    /// Topic configuration
    pub config: TopicConfig,
    /// Memory region for this topic
    pub memory_region: Arc<SharedMemoryRegion>,
    /// Memory region name for cleanup
    pub region_name: String,
    /// Ring buffer (either SPSC or MPMC)
    pub ring: TopicRingType,
    /// Buffer registry for large messages
    pub buffer_registry: Arc<BufferPoolRegistry>,
    /// Topic statistics
    pub stats: Arc<TopicStats>,
    /// Next publisher handle
    next_pub_handle: AtomicUsize,
    /// Next subscriber handle
    next_sub_handle: AtomicUsize,
}

impl TopicInstance {
    /// Create a new topic instance
    pub fn new(
        topic_id: TopicId,
        config: TopicConfig,
        memory_region: Arc<SharedMemoryRegion>,
        region_name: String,
        buffer_registry: Arc<BufferPoolRegistry>,
    ) -> Result<Self> {
        let stats = Arc::new(TopicStats::default());
        
        // Create appropriate ring buffer based on pattern
        let ring = match config.pattern {
            TopicPattern::SPSC | TopicPattern::SPMC => {
                TopicRingType::SPSC(Arc::new(SPSCTopicRing::new(config.ring_capacity * 1024, stats.clone())?))
            }
            TopicPattern::MPSC | TopicPattern::MPMC => {
                TopicRingType::MPMC(Arc::new(MPMCTopicRing::new(config.ring_capacity * 1024, stats.clone())?))
            }
        };

        Ok(Self {
            topic_id,
            config,
            memory_region,
            region_name,
            ring,
            buffer_registry,
            stats,
            next_pub_handle: AtomicUsize::new(1),
            next_sub_handle: AtomicUsize::new(1),
        })
    }

    /// Add a publisher to this topic
    pub fn add_publisher(&self) -> Result<PublisherHandle> {
        match self.config.pattern {
            TopicPattern::SPSC | TopicPattern::SPMC => {
                // Single producer patterns - only allow one publisher
                if self.stats.active_publishers.load(Ordering::Relaxed) > 0 {
                    return Err(RenoirError::invalid_parameter(
                        "pattern", 
                        "Topic only allows single publisher"
                    ));
                }
            }
            TopicPattern::MPSC | TopicPattern::MPMC => {
                // Multiple producer patterns - allow multiple publishers
            }
        }

        let handle = self.next_pub_handle.fetch_add(1, Ordering::SeqCst);
        self.stats.add_publisher();
        Ok(handle)
    }

    /// Remove a publisher from this topic
    pub fn remove_publisher(&self, _handle: PublisherHandle) -> Result<()> {
        self.stats.remove_publisher();
        Ok(())
    }

    /// Add a subscriber to this topic
    pub fn add_subscriber(&self) -> Result<SubscriberHandle> {
        match self.config.pattern {
            TopicPattern::SPSC | TopicPattern::MPSC => {
                // Single consumer patterns - only allow one subscriber
                if self.stats.active_subscribers.load(Ordering::Relaxed) > 0 {
                    return Err(RenoirError::invalid_parameter(
                        "pattern", 
                        "Topic only allows single subscriber"
                    ));
                }
            }
            TopicPattern::SPMC | TopicPattern::MPMC => {
                // Multiple consumer patterns - allow multiple subscribers
            }
        }

        let handle = self.next_sub_handle.fetch_add(1, Ordering::SeqCst);
        self.stats.add_subscriber();
        Ok(handle)
    }

    /// Remove a subscriber from this topic
    pub fn remove_subscriber(&self, _handle: SubscriberHandle) -> Result<()> {
        self.stats.remove_subscriber();
        Ok(())
    }

    /// Publish a message to this topic
    pub fn publish(&self, payload: Vec<u8>) -> Result<()> {
        let sequence = self.stats.current_sequence.load(Ordering::SeqCst);
        
        let message = if payload.len() > self.config.shared_pool_threshold && self.config.use_shared_pool {
            // Use shared buffer pool for large messages
            let descriptor = self.buffer_registry.get_buffer_for_payload(payload.len())?;
            let buffer_data = self.buffer_registry.get_buffer_data(&descriptor)?;
            
            // Copy payload to shared buffer
            unsafe {
                std::ptr::copy_nonoverlapping(
                    payload.as_ptr(),
                    buffer_data.as_mut_ptr(),
                    payload.len(),
                );
            }
            
            Message::new_descriptor(self.topic_id, sequence, descriptor)
        } else {
            // Store inline in ring buffer
            Message::new_inline(self.topic_id, sequence, payload)
        };

        match &self.ring {
            TopicRingType::SPSC(ring) => ring.try_publish(&message),
            TopicRingType::MPMC(ring) => ring.try_publish(&message),
        }
    }

    /// Subscribe to messages from this topic
    pub fn subscribe(&self) -> Result<Option<Message>> {
        match &self.ring {
            TopicRingType::SPSC(ring) => ring.try_consume(),
            TopicRingType::MPMC(ring) => ring.try_consume(),
        }
    }

    /// Wait for a message with timeout (Linux only)
    #[cfg(target_os = "linux")]
    pub fn wait_for_message(&self, timeout: Option<Duration>) -> Result<Option<Message>> {
        let timeout_ms = timeout.map(|d| d.as_millis() as u64);
        
        match &self.ring {
            TopicRingType::SPSC(ring) => ring.wait_for_message(timeout_ms),
            TopicRingType::MPMC(_ring) => {
                // MPMC doesn't support blocking wait, poll instead
                let start = Instant::now();
                loop {
                    if let Some(message) = self.subscribe()? {
                        return Ok(Some(message));
                    }
                    
                    if let Some(timeout) = timeout {
                        if start.elapsed() >= timeout {
                            break;
                        }
                    }
                    
                    thread::sleep(Duration::from_micros(100));
                }
                Ok(None)
            }
        }
    }

    /// Get notification file descriptor (Linux only)
    #[cfg(target_os = "linux")]
    pub fn notification_fd(&self) -> Option<std::os::fd::RawFd> {
        match &self.ring {
            TopicRingType::SPSC(ring) => ring.notification_fd(),
            TopicRingType::MPMC(ring) => ring.notification_fd(),
        }
    }
}

/// Enum for different ring buffer types
#[derive(Debug)]
pub enum TopicRingType {
    SPSC(Arc<SPSCTopicRing>),
    MPMC(Arc<MPMCTopicRing>),
}

/// Publisher handle for publishing messages to a topic
#[derive(Debug)]
pub struct Publisher {
    pub handle: PublisherHandle,
    pub topic_id: TopicId,
    topic: Arc<TopicInstance>,
}

impl Publisher {
    /// Publish a message to the topic
    pub fn publish(&self, payload: Vec<u8>) -> Result<()> {
        self.topic.publish(payload)
    }

    /// Get topic statistics
    pub fn stats(&self) -> Arc<TopicStats> {
        self.topic.stats.clone()
    }

    /// Get the topic configuration
    pub fn config(&self) -> &TopicConfig {
        &self.topic.config
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        let _ = self.topic.remove_publisher(self.handle);
    }
}

/// Subscriber handle for receiving messages from a topic
#[derive(Debug)]
pub struct Subscriber {
    pub handle: SubscriberHandle,
    pub topic_id: TopicId,
    topic: Arc<TopicInstance>,
    #[allow(dead_code)]
    last_sequence: AtomicUsize,
}

impl Subscriber {
    /// Subscribe to the next available message
    pub fn subscribe(&self) -> Result<Option<Message>> {
        self.topic.subscribe()
    }

    /// Wait for a message with optional timeout
    #[cfg(target_os = "linux")]
    pub fn wait_for_message(&self, timeout: Option<Duration>) -> Result<Option<Message>> {
        self.topic.wait_for_message(timeout)
    }

    /// Get notification file descriptor for external polling
    #[cfg(target_os = "linux")]
    pub fn notification_fd(&self) -> Option<std::os::fd::RawFd> {
        self.topic.notification_fd()
    }

    /// Get topic statistics
    pub fn stats(&self) -> Arc<TopicStats> {
        self.topic.stats.clone()
    }

    /// Get the topic configuration
    pub fn config(&self) -> &TopicConfig {
        &self.topic.config
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        let _ = self.topic.remove_subscriber(self.handle);
    }
}

/// Global topic manager statistics
#[derive(Debug, Default)]
pub struct TopicManagerStats {
    /// Total topics created
    pub topics_created: AtomicUsize,
    /// Total topics removed
    pub topics_removed: AtomicUsize,
    /// Total messages published across all topics
    pub total_messages_published: AtomicUsize,
    /// Total messages consumed across all topics
    pub total_messages_consumed: AtomicUsize,
    /// Peak message rate (messages/second)
    pub peak_message_rate: AtomicUsize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topic::{TopicPattern, TopicQoS, Reliability};


    #[test]
    fn test_topic_manager_creation() {
        let manager = TopicManager::new().unwrap();
        let stats = manager.manager_stats();
        assert_eq!(stats.topics_created.load(Ordering::Relaxed), 0);
    }

    #[test]  
    fn test_topic_creation_and_messaging() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "test_topic".to_string(),
            pattern: TopicPattern::SPSC,
            ring_capacity: 1024,
            max_payload_size: 1024,
            use_shared_pool: false,
            shared_pool_threshold: 4096,
            enable_notifications: true,
            qos: TopicQoS {
                durability: 1,
                reliability: Reliability::BestEffort,
                history_depth: 10,
            },
        };

        // Create topic
        let topic_id = manager.create_topic(config).unwrap();
        assert!(topic_id > 0);

        // Create publisher and subscriber
        let publisher = manager.create_publisher("test_topic").unwrap();
        let subscriber = manager.create_subscriber("test_topic").unwrap();

        // Publish a message
        let payload = b"Hello, Renoir Topics!".to_vec();
        publisher.publish(payload.clone()).unwrap();

        // Subscribe to the message
        let received = subscriber.subscribe().unwrap();
        assert!(received.is_some());

        let message = received.unwrap();
        let header_topic_id = message.header.topic_id;
        assert_eq!(header_topic_id, topic_id);
        
        match message.payload {
            crate::topic::MessagePayload::Inline(data) => {
                assert_eq!(data, payload);
            }
            _ => panic!("Expected inline payload"),
        }

        // Verify statistics
        let stats = subscriber.stats();
        assert_eq!(stats.messages_published.load(Ordering::Relaxed), 1);
        assert_eq!(stats.messages_consumed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_spsc_pattern_enforcement() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "spsc_topic".to_string(),
            pattern: TopicPattern::SPSC,
            ..Default::default()
        };

        manager.create_topic(config).unwrap();

        // First publisher should succeed
        let _pub1 = manager.create_publisher("spsc_topic").unwrap();
        
        // Second publisher should fail
        let pub2_result = manager.create_publisher("spsc_topic");
        assert!(pub2_result.is_err());

        // First subscriber should succeed
        let _sub1 = manager.create_subscriber("spsc_topic").unwrap();
        
        // Second subscriber should fail
        let sub2_result = manager.create_subscriber("spsc_topic");
        assert!(sub2_result.is_err());
    }

    #[test]
    fn test_large_message_with_shared_pools() {
        let manager = TopicManager::new().unwrap();
        
        let config = TopicConfig {
            name: "large_msg_topic".to_string(),
            pattern: TopicPattern::SPSC,
            use_shared_pool: true,
            shared_pool_threshold: 1024,
            max_payload_size: 8192,
            ..Default::default()
        };

        manager.create_topic(config).unwrap();

        let publisher = manager.create_publisher("large_msg_topic").unwrap();
        let subscriber = manager.create_subscriber("large_msg_topic").unwrap();

        // Publish a large message (will use shared pool)
        let large_payload = vec![0x42u8; 2048];
        publisher.publish(large_payload.clone()).unwrap();

        // Subscribe and verify
        let received = subscriber.subscribe().unwrap();
        assert!(received.is_some());

        let message = received.unwrap();
        match message.payload {
            crate::topic::MessagePayload::Descriptor(desc) => {
                // Verify descriptor properties
                assert_eq!(desc.payload_size, 2048);
                assert!(desc.pool_id > 0);
            }
            _ => panic!("Expected descriptor payload for large message"),
        }
    }

    #[test]
    fn test_topic_removal() {
        let manager = TopicManager::new().unwrap();

        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let topic_name = format!("removable_topic_{}_{}", 
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let config = TopicConfig {
            name: topic_name.clone(),
            ..Default::default()
        };

        manager.create_topic(config).unwrap();

        // Should be able to remove topic with no publishers/subscribers
        manager.remove_topic(&topic_name).unwrap();

        // Recreate topic
        let config = TopicConfig {
            name: topic_name.clone(),
            ..Default::default()
        };
        manager.create_topic(config).unwrap();        let _publisher = manager.create_publisher(&topic_name).unwrap();
        
        // Should not be able to remove topic with active publisher
        let result = manager.remove_topic(&topic_name);
        assert!(result.is_err());
    }
}