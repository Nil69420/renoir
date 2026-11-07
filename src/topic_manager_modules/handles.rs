//! Publisher and Subscriber handles for topic communication

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use parking_lot::Mutex;

use crate::{
    error::Result,
    topic::{Message, TopicStats},
    topic_rings::{BroadcastCursor, BroadcastReservation, ReadInto},
};

use super::{
    instance::{PublisherHandle, SubscriberHandle, TopicInstance},
    manager::TopicId,
};

/// Publisher handle for publishing messages to a topic
#[derive(Debug)]
pub struct Publisher {
    pub handle: PublisherHandle,
    pub topic_id: TopicId,
    topic: Arc<TopicInstance>,
}

impl Publisher {
    /// Create a new publisher (internal use)
    pub(super) fn new(
        handle: PublisherHandle,
        topic_id: TopicId,
        topic: Arc<TopicInstance>,
    ) -> Self {
        Self {
            handle,
            topic_id,
            topic,
        }
    }

    /// Publish a message to the topic
    pub fn publish(&self, payload: Vec<u8>) -> Result<()> {
        if payload.len() > self.topic.config.max_payload_size {
            return self.topic.publish_large(payload);
        }

        self.topic.publish(payload)
    }

    /// Publish a message if there's space in the ring buffer
    pub fn try_publish(&self, payload: Vec<u8>) -> Result<bool> {
        if self.topic.is_full() {
            return Ok(false);
        }

        self.publish(payload)?;
        Ok(true)
    }

    pub fn supports_zero_copy(&self) -> bool {
        self.topic.supports_zero_copy()
    }

    pub fn reserve(&self, len: usize) -> Result<BroadcastReservation> {
        self.topic.reserve(len)
    }

    pub fn commit(&self, reservation: BroadcastReservation, len: usize) -> Result<u64> {
        self.topic.commit_reserved(reservation, len)
    }

    pub fn discard(&self, reservation: BroadcastReservation) {
        self.topic.discard_reserved(reservation)
    }

    /// Get topic statistics
    pub fn stats(&self) -> Arc<TopicStats> {
        self.topic.stats.clone()
    }

    /// Get the topic name
    pub fn topic_name(&self) -> &str {
        &self.topic.config.name
    }

    /// Check if the ring buffer is full
    pub fn is_full(&self) -> bool {
        self.topic.is_full()
    }

    /// Get the number of pending messages in the ring buffer
    pub fn pending_messages(&self) -> usize {
        self.topic.pending_messages()
    }

    /// Get capacity of the ring buffer
    pub fn capacity(&self) -> usize {
        self.topic.config.ring_capacity
    }

    /// Check if the topic is active (has subscribers)
    pub fn has_subscribers(&self) -> bool {
        self.topic.stats.active_subscribers.load(Ordering::Relaxed) > 0
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
    last_sequence: AtomicUsize,
    cursor: Mutex<Option<BroadcastCursor>>,
}

impl Subscriber {
    /// Create a new subscriber (internal use)
    pub(super) fn new(
        handle: SubscriberHandle,
        topic_id: TopicId,
        topic: Arc<TopicInstance>,
    ) -> Self {
        Self {
            handle,
            topic_id,
            cursor: Mutex::new(topic.new_cursor()),
            topic,
            last_sequence: AtomicUsize::new(0),
        }
    }

    /// Subscribe to the next available message
    pub fn subscribe(&self) -> Result<Option<Message>> {
        let message = self.topic.subscribe_with(&self.cursor)?;

        if let Some(ref msg) = message {
            self.last_sequence
                .store(msg.header.sequence as usize, Ordering::Relaxed);
        }

        Ok(message)
    }

    /// Try to get a message without blocking
    pub fn try_subscribe(&self) -> Result<Option<Message>> {
        self.subscribe()
    }

    /// Peek at the next message without consuming it
    /// Returns a copy of the message without removing it from the queue
    /// Note: In MPMC mode, the message may be consumed by another subscriber
    /// between peek and the next subscribe call
    pub fn peek(&self) -> Result<Option<Message>> {
        self.topic.peek_with(&self.cursor)
    }

    pub fn read_into(&self, dst: &mut [u8]) -> Result<ReadInto> {
        let result = self.topic.read_into_with(&self.cursor, dst)?;
        if let ReadInto::Message { sequence, .. } = result {
            self.last_sequence
                .store(sequence as usize, Ordering::Relaxed);
        }
        Ok(result)
    }

    pub fn skip_to_latest(&self, keep: u64) -> u64 {
        self.topic.skip_to_latest_with(&self.cursor, keep)
    }

    pub fn wait_readable(&self, timeout: Option<Duration>) -> bool {
        self.topic.wait_readable_with(&self.cursor, timeout)
    }

    pub fn overruns(&self) -> u64 {
        self.cursor.lock().map_or(0, |c| c.overruns())
    }

    /// Wait for a message with optional timeout
    #[cfg(target_os = "linux")]
    pub fn wait_for_message(&self, timeout: Option<Duration>) -> Result<Option<Message>> {
        let message = self.topic.wait_with(&self.cursor, timeout)?;

        if let Some(ref msg) = message {
            self.last_sequence
                .store(msg.header.sequence as usize, Ordering::Relaxed);
        }

        Ok(message)
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

    /// Get the topic name
    pub fn topic_name(&self) -> &str {
        &self.topic.config.name
    }

    /// Check if the ring buffer is empty
    pub fn is_empty(&self) -> bool {
        self.topic.is_empty()
    }

    /// Get the number of pending messages in the ring buffer
    pub fn pending_messages(&self) -> usize {
        self.topic.pending_messages()
    }

    /// Get the last consumed message sequence number
    pub fn last_sequence(&self) -> usize {
        self.last_sequence.load(Ordering::Relaxed)
    }

    /// Check if there are new messages since last consumption
    pub fn has_new_messages(&self) -> bool {
        let current_seq = self.topic.stats.current_sequence.load(Ordering::Relaxed);
        let last_seq = self.last_sequence.load(Ordering::Relaxed);
        current_seq > last_seq as u64
    }

    /// Check if the topic is active (has publishers)
    pub fn has_publishers(&self) -> bool {
        self.topic.stats.active_publishers.load(Ordering::Relaxed) > 0
    }

    /// Drain all pending messages
    pub fn drain_messages(&self) -> Result<Vec<Message>> {
        let mut messages = Vec::new();

        while let Some(message) = self.subscribe()? {
            messages.push(message);
        }

        Ok(messages)
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        let _ = self.topic.remove_subscriber(self.handle);
    }
}
