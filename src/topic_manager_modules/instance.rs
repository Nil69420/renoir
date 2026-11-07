//! Individual topic instance with appropriate ring buffer and configuration

use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use crate::{
    error::{RenoirError, Result},
    memory::SharedMemoryRegion,
    shared_pools::BufferPoolRegistry,
    topic::{Message, MessagePayload, TopicConfig, TopicPattern, TopicStats},
    topic_rings::{BroadcastCursor, BroadcastTopicRing, MPMCTopicRing, SPSCTopicRing},
};

use super::manager::TopicId;

/// Handle for publishers
pub type PublisherHandle = usize;

/// Handle for subscribers  
pub type SubscriberHandle = usize;

/// Individual topic instance with appropriate ring buffer and configuration
#[derive(Debug)]
pub struct TopicInstance {
    pub topic_id: TopicId,
    pub config: TopicConfig,
    pub memory_region: Arc<SharedMemoryRegion>,
    pub region_name: String,
    pub ring: TopicRingType,
    pub buffer_registry: Arc<BufferPoolRegistry>,
    pub stats: Arc<TopicStats>,
    next_pub_handle: AtomicUsize,
    next_sub_handle: AtomicUsize,
    /// Active publisher handles
    publisher_handles: Mutex<HashSet<PublisherHandle>>,
    /// Active subscriber handles
    subscriber_handles: Mutex<HashSet<SubscriberHandle>>,
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

        let ring_capacity = config.ring_capacity * 1024;

        // Place the ring inside the shared memory region so that all
        // processes attaching to the region share the same ring state
        let region_memory = memory_region.as_ptr::<u8>();
        let region_size = memory_region.size();
        let memory = std::ptr::NonNull::new(region_memory as *mut u8)
            .ok_or_else(|| RenoirError::memory("Shared memory region returned a null pointer"))?;

        // Create appropriate ring buffer based on pattern
        let ring = match config.pattern {
            TopicPattern::SPSC => {
                // SAFETY: region memory is valid, aligned, and outlives the ring
                // (TopicInstance holds the Arc to the region)
                let ring = unsafe {
                    SPSCTopicRing::from_memory(memory, region_size, ring_capacity, stats.clone())?
                };
                TopicRingType::SPSC(Arc::new(ring))
            }
            TopicPattern::SPMC => {
                if config.use_shared_pool {
                    return Err(RenoirError::invalid_parameter(
                        "use_shared_pool",
                        "Broadcast (SPMC) topics carry inline payloads only",
                    ));
                }
                let ring = unsafe {
                    BroadcastTopicRing::from_memory(
                        memory,
                        region_size,
                        ring_capacity,
                        config.max_payload_size,
                        stats.clone(),
                    )?
                };
                TopicRingType::Broadcast(Arc::new(ring))
            }
            TopicPattern::MPSC | TopicPattern::MPMC => {
                // SAFETY: region memory is valid, aligned, and outlives the ring
                // (TopicInstance holds the Arc to the region)
                let ring = unsafe {
                    MPMCTopicRing::from_memory(memory, region_size, ring_capacity, stats.clone())?
                };
                TopicRingType::MPMC(Arc::new(ring))
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
            publisher_handles: Mutex::new(HashSet::new()),
            subscriber_handles: Mutex::new(HashSet::new()),
        })
    }

    /// Add a publisher to this topic
    pub fn add_publisher(&self) -> Result<PublisherHandle> {
        let single_producer =
            matches!(self.config.pattern, TopicPattern::SPSC | TopicPattern::SPMC);
        if single_producer {
            // The 0 -> 1 CAS makes the check-and-attach atomic so two racing
            // threads cannot both attach, which would violate the SPSC ring's
            // single-writer invariant.
            match self.stats.active_publishers.compare_exchange(
                0,
                1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {}
                Err(_) => {
                    return Err(RenoirError::invalid_parameter(
                        "pattern",
                        "Topic only allows single publisher",
                    ))
                }
            }
        } else {
            self.stats.add_publisher();
        }

        let handle = self.next_pub_handle.fetch_add(1, Ordering::SeqCst);
        self.publisher_handles.lock().insert(handle);
        Ok(handle)
    }

    pub fn remove_publisher(&self, handle: PublisherHandle) -> Result<()> {
        if !self.publisher_handles.lock().remove(&handle) {
            return Err(RenoirError::invalid_parameter(
                "handle",
                "Publisher handle not found",
            ));
        }
        self.stats.remove_publisher();
        Ok(())
    }

    /// Add a subscriber to this topic
    pub fn add_subscriber(&self) -> Result<SubscriberHandle> {
        let single_consumer =
            matches!(self.config.pattern, TopicPattern::SPSC | TopicPattern::MPSC);
        if single_consumer {
            // CAS makes the check-and-attach atomic (see add_publisher)
            match self.stats.active_subscribers.compare_exchange(
                0,
                1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {}
                Err(_) => {
                    return Err(RenoirError::invalid_parameter(
                        "pattern",
                        "Topic only allows single subscriber",
                    ))
                }
            }
        } else {
            self.stats.add_subscriber();
        }

        let handle = self.next_sub_handle.fetch_add(1, Ordering::SeqCst);
        self.subscriber_handles.lock().insert(handle);
        Ok(handle)
    }

    pub fn remove_subscriber(&self, handle: SubscriberHandle) -> Result<()> {
        if !self.subscriber_handles.lock().remove(&handle) {
            return Err(RenoirError::invalid_parameter(
                "handle",
                "Subscriber handle not found",
            ));
        }
        self.stats.remove_subscriber();
        Ok(())
    }

    /// Publish a message to this topic
    pub fn publish(&self, payload: Vec<u8>) -> Result<()> {
        let sequence = self.stats.current_sequence.fetch_add(1, Ordering::SeqCst) + 1;

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

                drop(buffer_data);

                Message::new_descriptor(self.topic_id, sequence, descriptor)
            } else {
                // Store inline in ring buffer
                Message::new_inline(self.topic_id, sequence, payload)
            };

        let result = match &self.ring {
            TopicRingType::SPSC(ring) => ring.try_publish(&message),
            TopicRingType::MPMC(ring) => ring.try_publish(&message),
            TopicRingType::Broadcast(ring) => ring.try_publish(&message),
        };

        // Return pool buffer on ring-full to prevent leak
        if result.is_err() {
            if let MessagePayload::Descriptor(ref desc) = message.payload {
                let _ = self.buffer_registry.return_buffer(desc);
            }
        }

        result
    }

    /// Publish a large payload by wrapping it with a BlobHeader.
    ///
    /// Called automatically when `payload.len() > config.max_payload_size`.
    /// The BlobHeader is detected and stripped transparently on the receive side.
    pub fn publish_large(&self, payload: Vec<u8>) -> Result<()> {
        use crate::large_payloads::blob::{content_types, BlobHeader, BlobManager};

        let blob_mgr = BlobManager::new();
        let header = blob_mgr.create_header(content_types::CUSTOM_BINARY, &payload);

        let mut blob_data = Vec::with_capacity(BlobHeader::SIZE + payload.len());
        // SAFETY: BlobHeader is #[repr(C, packed)]
        let header_bytes = unsafe {
            std::slice::from_raw_parts(&header as *const BlobHeader as *const u8, BlobHeader::SIZE)
        };
        blob_data.extend_from_slice(header_bytes);
        blob_data.extend_from_slice(&payload);

        let sequence = self.stats.current_sequence.fetch_add(1, Ordering::SeqCst) + 1;

        let message = if self.config.use_shared_pool {
            let descriptor = self
                .buffer_registry
                .get_buffer_for_payload(blob_data.len())?;
            let buffer_data = self.buffer_registry.get_buffer_data(&descriptor)?;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    blob_data.as_ptr(),
                    buffer_data.as_mut_ptr(),
                    blob_data.len(),
                );
            }

            drop(buffer_data);

            Message::new_descriptor(self.topic_id, sequence, descriptor)
        } else {
            Message::new_inline(self.topic_id, sequence, blob_data)
        };

        let result = match &self.ring {
            TopicRingType::SPSC(ring) => ring.try_publish(&message),
            TopicRingType::MPMC(ring) => ring.try_publish(&message),
            TopicRingType::Broadcast(ring) => ring.try_publish(&message),
        };

        // Return pool buffer on ring-full to prevent leak
        if result.is_err() {
            if let MessagePayload::Descriptor(ref desc) = message.payload {
                let _ = self.buffer_registry.return_buffer(desc);
            }
        }

        result
    }

    /// Subscribe to messages from this topic
    pub fn subscribe(&self) -> Result<Option<Message>> {
        match &self.ring {
            TopicRingType::SPSC(ring) => ring.try_consume(),
            TopicRingType::MPMC(ring) => ring.try_consume(),
            TopicRingType::Broadcast(_) => Err(RenoirError::invalid_parameter(
                "pattern",
                "Broadcast topics require a per-subscriber cursor",
            )),
        }
    }

    pub fn new_cursor(&self) -> Option<BroadcastCursor> {
        match &self.ring {
            TopicRingType::Broadcast(ring) => Some(ring.cursor_latest()),
            _ => None,
        }
    }

    pub fn subscribe_with(
        &self,
        cursor: &Mutex<Option<BroadcastCursor>>,
    ) -> Result<Option<Message>> {
        match (&self.ring, &mut *cursor.lock()) {
            (TopicRingType::Broadcast(ring), Some(c)) => ring.try_consume(c),
            _ => self.subscribe(),
        }
    }

    pub fn peek_with(&self, cursor: &Mutex<Option<BroadcastCursor>>) -> Result<Option<Message>> {
        match (&self.ring, &*cursor.lock()) {
            (TopicRingType::Broadcast(ring), Some(c)) => ring.try_peek(c),
            _ => self.peek(),
        }
    }

    pub fn wait_with(
        &self,
        cursor: &Mutex<Option<BroadcastCursor>>,
        timeout: Option<Duration>,
    ) -> Result<Option<Message>> {
        if cursor.lock().is_none() {
            return self.wait_for_message(timeout);
        }
        let start = Instant::now();
        loop {
            if let Some(message) = self.subscribe_with(cursor)? {
                return Ok(Some(message));
            }
            if let Some(timeout) = timeout {
                if start.elapsed() >= timeout {
                    return Ok(None);
                }
            }
            thread::sleep(Duration::from_micros(50));
        }
    }

    /// Peek at the next message without consuming it
    /// Returns a copy of the message without removing it from the queue
    pub fn peek(&self) -> Result<Option<Message>> {
        match &self.ring {
            TopicRingType::SPSC(ring) => ring.try_peek(),
            TopicRingType::MPMC(ring) => ring.try_peek(),
            TopicRingType::Broadcast(_) => Ok(None),
        }
    }

    /// Wait for a message with timeout (Linux only)
    #[cfg(target_os = "linux")]
    pub fn wait_for_message(&self, timeout: Option<Duration>) -> Result<Option<Message>> {
        let timeout_ms = timeout.map(|d| d.as_millis() as u64);

        match &self.ring {
            TopicRingType::SPSC(ring) => ring.wait_for_message(timeout_ms),
            TopicRingType::MPMC(_) | TopicRingType::Broadcast(_) => {
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
            TopicRingType::Broadcast(_) => None,
        }
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
    Broadcast(Arc<BroadcastTopicRing>),
}
