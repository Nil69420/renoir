//! Single Producer Single Consumer ring buffer for topic messaging

use std::{
    os::fd::RawFd,
    ptr::NonNull,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use crate::{
    error::{RenoirError, Result},
    sync::MioEventNotification,
    topic::{Message, MessageDescriptor, MessageHeader, TopicStats},
};

/// Single Producer Single Consumer ring buffer optimized for topic messaging
#[derive(Debug)]
pub struct SPSCTopicRing {
    /// Buffer storage for messages
    buffer: NonNull<u8>,
    /// Total buffer capacity in bytes  
    capacity: usize,
    /// Write position (producer only)
    write_pos: AtomicUsize,
    /// Read position (consumer only)
    read_pos: AtomicUsize,
    /// Cached read position for producer (reduces cache misses)
    cached_read_pos: AtomicUsize,
    /// Cached write position for consumer (reduces cache misses)
    cached_write_pos: AtomicUsize,
    /// Topic statistics
    stats: Arc<TopicStats>,
    /// Mio-based event notification system
    notifier: Arc<MioEventNotification>,
}

impl SPSCTopicRing {
    /// Create a new SPSC topic ring
    pub fn new(capacity: usize, stats: Arc<TopicStats>) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0",
            ));
        }

        let layout = std::alloc::Layout::array::<u8>(capacity)
            .map_err(|_| RenoirError::memory("Failed to create layout for topic ring"))?;

        let buffer = unsafe {
            let ptr = std::alloc::alloc(layout);
            NonNull::new(ptr).ok_or_else(|| RenoirError::memory("Failed to allocate topic ring"))?
        };

        let notifier = MioEventNotification::new()
            .map_err(|_| RenoirError::memory("Failed to create event notifier"))?;

        Ok(Self {
            buffer,
            capacity,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            cached_read_pos: AtomicUsize::new(0),
            cached_write_pos: AtomicUsize::new(0),
            stats,
            notifier,
        })
    }

    /// Create from existing shared memory
    pub unsafe fn from_memory(
        memory: NonNull<u8>,
        capacity: usize,
        stats: Arc<TopicStats>,
    ) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0",
            ));
        }

        let notifier = MioEventNotification::new()
            .map_err(|_| RenoirError::memory("Failed to create event notifier"))?;

        Ok(Self {
            buffer: memory,
            capacity,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            cached_read_pos: AtomicUsize::new(0),
            cached_write_pos: AtomicUsize::new(0),
            stats,
            notifier,
        })
    }

    /// Get the buffer capacity
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get available space for writing (bytes)
    pub fn available_write_space(&self) -> usize {
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let cached_read = self.cached_read_pos.load(Ordering::Relaxed);

        // If we don't have enough space, refresh the cached read position
        if write_pos.wrapping_sub(cached_read) >= self.capacity {
            let fresh_read = self.read_pos.load(Ordering::Acquire);
            self.cached_read_pos.store(fresh_read, Ordering::Relaxed);
            self.capacity - write_pos.wrapping_sub(fresh_read)
        } else {
            self.capacity - write_pos.wrapping_sub(cached_read)
        }
    }

    /// Try to publish a message (producer side)
    pub fn try_publish(&self, message: &Message) -> Result<()> {
        let message_size = message.total_size();

        // Check if we have enough space
        if self.available_write_space() < message_size {
            self.stats.record_dropped();
            return Err(RenoirError::buffer_full("Topic ring is full"));
        }

        // Serialize and write the message
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let buffer_pos = write_pos & (self.capacity - 1);

        unsafe {
            let write_ptr = self.buffer.as_ptr().add(buffer_pos);
            self.serialize_message(message, write_ptr, message_size)?;
        }

        // Update write position
        let new_write_pos = write_pos.wrapping_add(message_size);
        self.write_pos.store(new_write_pos, Ordering::Release);

        // Record statistics
        let _sequence = self.stats.record_published(message_size);

        // Notify readers if enabled
        if self.notifier.is_enabled() {
            let _ = self.notifier.notify(); // Ignore notification errors
        }

        Ok(())
    }

    /// Try to consume a message (consumer side)
    pub fn try_consume(&self) -> Result<Option<Message>> {
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let cached_write = self.cached_write_pos.load(Ordering::Relaxed);

        // Check if there's data available
        if read_pos == cached_write {
            // Refresh cached write position
            let fresh_write = self.write_pos.load(Ordering::Acquire);
            self.cached_write_pos.store(fresh_write, Ordering::Relaxed);

            if read_pos == fresh_write {
                return Ok(None); // No data available
            }
        }

        // Read message header first
        let buffer_pos = read_pos & (self.capacity - 1);
        let header = unsafe {
            let read_ptr = self.buffer.as_ptr().add(buffer_pos);
            self.deserialize_header(read_ptr)?
        };

        let message_size = MessageHeader::SIZE + header.payload_length as usize;

        // Check if we have the complete message
        let available = self
            .cached_write_pos
            .load(Ordering::Relaxed)
            .wrapping_sub(read_pos);
        if available < message_size {
            return Ok(None); // Incomplete message
        }

        // Read the complete message
        let message = unsafe {
            let read_ptr = self.buffer.as_ptr().add(buffer_pos);
            self.deserialize_message(&header, read_ptr)?
        };

        // Update read position
        self.read_pos
            .store(read_pos.wrapping_add(message_size), Ordering::Release);

        // Record statistics
        self.stats.record_consumed();

        Ok(Some(message))
    }

    /// Peek at the next message without consuming it
    /// Returns a copy of the message at the head of the queue without advancing the read position
    pub fn try_peek(&self) -> Result<Option<Message>> {
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let cached_write = self.cached_write_pos.load(Ordering::Relaxed);

        // Check if there's data available
        if read_pos == cached_write {
            // Refresh cached write position
            let fresh_write = self.write_pos.load(Ordering::Acquire);
            self.cached_write_pos.store(fresh_write, Ordering::Relaxed);

            if read_pos == fresh_write {
                return Ok(None); // No data available
            }
        }

        // Read message header first
        let buffer_pos = read_pos & (self.capacity - 1);
        let header = unsafe {
            let read_ptr = self.buffer.as_ptr().add(buffer_pos);
            self.deserialize_header(read_ptr)?
        };

        let message_size = MessageHeader::SIZE + header.payload_length as usize;

        // Check if we have the complete message
        let available = self
            .cached_write_pos
            .load(Ordering::Relaxed)
            .wrapping_sub(read_pos);
        if available < message_size {
            return Ok(None); // Incomplete message
        }

        // Read the complete message (but don't advance read position)
        let message = unsafe {
            let read_ptr = self.buffer.as_ptr().add(buffer_pos);
            self.deserialize_message(&header, read_ptr)?
        };

        // Note: read_pos is NOT updated - this is the key difference from try_consume
        Ok(Some(message))
    }

    /// Wait for a message with optional timeout (blocking)
    #[cfg(target_os = "linux")]
    pub fn wait_for_message(&self, timeout_ms: Option<u64>) -> Result<Option<Message>> {
        if let Some(message) = self.try_consume()? {
            return Ok(Some(message));
        }

        // Wait using mio's async polling
        let _ = self.notifier.wait_async(timeout_ms);
        self.try_consume()
    }

    unsafe fn serialize_message(
        &self,
        message: &Message,
        ptr: *mut u8,
        _size: usize,
    ) -> Result<()> {
        // Write header
        std::ptr::copy_nonoverlapping(
            &message.header as *const MessageHeader as *const u8,
            ptr,
            MessageHeader::SIZE,
        );

        // Write payload
        let payload_ptr = ptr.add(MessageHeader::SIZE);
        match &message.payload {
            crate::topic::MessagePayload::Inline(data) => {
                std::ptr::copy_nonoverlapping(data.as_ptr(), payload_ptr, data.len());
            }
            crate::topic::MessagePayload::Descriptor(desc) => {
                std::ptr::copy_nonoverlapping(
                    desc as *const MessageDescriptor as *const u8,
                    payload_ptr,
                    std::mem::size_of::<MessageDescriptor>(),
                );
            }
        }

        Ok(())
    }

    unsafe fn deserialize_header(&self, ptr: *const u8) -> Result<MessageHeader> {
        let header: MessageHeader = std::ptr::read_unaligned(ptr as *const MessageHeader);
        header.validate()?;
        Ok(header)
    }

    unsafe fn deserialize_message(
        &self,
        header: &MessageHeader,
        ptr: *const u8,
    ) -> Result<Message> {
        let payload_ptr = ptr.add(MessageHeader::SIZE);

        let payload = if header.payload_length <= std::mem::size_of::<MessageDescriptor>() as u32 {
            // This might be a descriptor
            let desc: MessageDescriptor =
                std::ptr::read_unaligned(payload_ptr as *const MessageDescriptor);
            crate::topic::MessagePayload::Descriptor(desc)
        } else {
            // Inline payload
            let mut data = vec![0u8; header.payload_length as usize];
            std::ptr::copy_nonoverlapping(
                payload_ptr,
                data.as_mut_ptr(),
                header.payload_length as usize,
            );
            crate::topic::MessagePayload::Inline(data)
        };

        Ok(Message {
            header: *header,
            payload,
        })
    }

    /// Get the eventfd for external polling
    #[cfg(target_os = "linux")]
    pub fn notification_fd(&self) -> Option<RawFd> {
        self.notifier.event_fd()
    }

    /// Enable or disable notifications
    pub fn set_notifications(&self, enabled: bool) {
        self.notifier.set_enabled(enabled);
    }

    /// Get current buffer utilization (0.0 to 1.0)
    pub fn utilization(&self) -> f32 {
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let used = write_pos.wrapping_sub(read_pos);
        used as f32 / self.capacity as f32
    }

    /// Reset the ring buffer
    pub fn reset(&self) {
        self.write_pos.store(0, Ordering::Release);
        self.read_pos.store(0, Ordering::Release);
        self.cached_read_pos.store(0, Ordering::Relaxed);
        self.cached_write_pos.store(0, Ordering::Relaxed);
    }
}

impl Drop for SPSCTopicRing {
    fn drop(&mut self) {
        // MioEventNotification will automatically clean up eventfd on drop

        // Deallocate buffer if we own it
        let layout = std::alloc::Layout::array::<u8>(self.capacity).unwrap();
        unsafe {
            std::alloc::dealloc(self.buffer.as_ptr(), layout);
        }
    }
}

unsafe impl Send for SPSCTopicRing {}
unsafe impl Sync for SPSCTopicRing {}
