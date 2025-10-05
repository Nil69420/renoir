//! Individual topic instance with appropriate ring buffer and configuration

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    error::{RenoirError, Result},
    memory::SharedMemoryRegion,
    shared_pools::BufferPoolRegistry,
    topic::{Message, TopicConfig, TopicPattern, TopicStats},
    topic_rings::{MPMCTopicRing, SPSCTopicRing},
};

use super::manager::TopicId;

/// Handle for publishers
pub type PublisherHandle = usize;

/// Handle for subscribers  
pub type SubscriberHandle = usize;

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
            TopicPattern::SPSC | TopicPattern::SPMC => TopicRingType::SPSC(Arc::new(
                SPSCTopicRing::new(config.ring_capacity * 1024, stats.clone())?,
            )),
            TopicPattern::MPSC | TopicPattern::MPMC => TopicRingType::MPMC(Arc::new(
                MPMCTopicRing::new(config.ring_capacity * 1024, stats.clone())?,
            )),
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
                        "Topic only allows single publisher",
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
                        "Topic only allows single subscriber",
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

        let message =
            if payload.len() > self.config.shared_pool_threshold && self.config.use_shared_pool {
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

    /// Peek at the next message without consuming it
    /// Returns a copy of the message without removing it from the queue
    pub fn peek(&self) -> Result<Option<Message>> {
        match &self.ring {
            TopicRingType::SPSC(ring) => ring.try_peek(),
            TopicRingType::MPMC(ring) => ring.try_peek(),
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

    /// Check if the topic is active (has publishers and subscribers)
    pub fn is_active(&self) -> bool {
        self.stats.active_publishers.load(Ordering::Relaxed) > 0
            && self.stats.active_subscribers.load(Ordering::Relaxed) > 0
    }

    /// Get the number of messages in the ring buffer
    pub fn pending_messages(&self) -> usize {
        // Note: TopicRings don't expose direct size method
        // This could be calculated from write/read positions but not available in current interface
        0
    }

    /// Check if the ring buffer is full
    pub fn is_full(&self) -> bool {
        // Note: TopicRings don't expose is_full method
        // Could check if available_write_space is 0, but not available in current interface
        false
    }

    /// Check if the ring buffer is empty
    pub fn is_empty(&self) -> bool {
        // Note: TopicRings don't expose is_empty method
        // Could check if no pending messages, but not available in current interface
        true
    }
}

/// Enum for different ring buffer types
#[derive(Debug)]
pub enum TopicRingType {
    SPSC(Arc<SPSCTopicRing>),
    MPMC(Arc<MPMCTopicRing>),
}
