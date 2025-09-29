//! Multi Producer Multi Consumer ring buffer for topic messaging

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    ptr::NonNull,
    os::fd::RawFd,
};

use crate::{
    error::{RenoirError, Result},
    topic::{Message, MessageHeader, TopicStats, MessageDescriptor},
};

/// Multi-Producer Multi-Consumer ring buffer for topics with multiple writers/readers
#[derive(Debug)]
pub struct MPMCTopicRing {
    /// Buffer storage
    buffer: NonNull<u8>,
    /// Buffer capacity in bytes
    capacity: usize,
    /// Write position (atomic for multiple producers)
    write_pos: AtomicUsize,
    /// Read position (atomic for multiple consumers) 
    read_pos: AtomicUsize,
    /// Topic statistics
    stats: Arc<TopicStats>,
    /// Notification file descriptor (Linux only)
    #[cfg(target_os = "linux")]
    notify_fd: Option<std::os::fd::OwnedFd>,
}

impl MPMCTopicRing {
    /// Create a new MPMC topic ring
    pub fn new(capacity: usize, stats: Arc<TopicStats>) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0"
            ));
        }

        let layout = std::alloc::Layout::array::<u8>(capacity)
            .map_err(|_| RenoirError::memory("Failed to create layout for MPMC ring"))?;

        let buffer = unsafe {
            let ptr = std::alloc::alloc(layout);
            NonNull::new(ptr).ok_or_else(|| RenoirError::memory("Failed to allocate MPMC ring"))?
        };

        #[cfg(target_os = "linux")]
        let notify_fd = Self::create_eventfd()?;

        Ok(Self {
            buffer,
            capacity,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            stats,
            #[cfg(target_os = "linux")]
            notify_fd,
        })
    }

    #[cfg(target_os = "linux")]
    fn create_eventfd() -> Result<Option<std::os::fd::OwnedFd>> {
        use nix::sys::eventfd::{eventfd, EfdFlags};
        
        let fd = eventfd(0, EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_NONBLOCK)
            .map_err(|_| RenoirError::memory("Failed to create eventfd"))?;
        Ok(Some(fd))
    }

    /// Get the buffer capacity
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Try to publish a message (thread-safe for multiple producers)
    pub fn try_publish(&self, message: &Message) -> Result<()> {
        let message_size = message.total_size();
        
        // Atomic allocation of write space
        let write_start = loop {
            let current_write = self.write_pos.load(Ordering::Relaxed);
            let current_read = self.read_pos.load(Ordering::Acquire);
            
            // Check if we have enough space
            if current_write.wrapping_sub(current_read) + message_size > self.capacity {
                self.stats.record_dropped();
                return Err(RenoirError::buffer_full("MPMC ring is full"));
            }
            
            // Try to atomically reserve space
            match self.write_pos.compare_exchange_weak(
                current_write,
                current_write.wrapping_add(message_size),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break current_write,
                Err(_) => continue, // Retry on contention
            }
        };

        // Write the message
        let buffer_pos = write_start & (self.capacity - 1);
        unsafe {
            let write_ptr = self.buffer.as_ptr().add(buffer_pos);
            self.serialize_message(message, write_ptr, message_size)?;
        }

        // Record statistics
        let _sequence = self.stats.record_published(message_size);

        // Notify readers
        self.notify_readers()?;

        Ok(())
    }

    /// Try to consume a message (thread-safe for multiple consumers)
    pub fn try_consume(&self) -> Result<Option<Message>> {
        loop {
            let current_read = self.read_pos.load(Ordering::Relaxed);
            let current_write = self.write_pos.load(Ordering::Acquire);
            
            if current_read == current_write {
                return Ok(None); // No data available
            }

            // Read message header
            let buffer_pos = current_read & (self.capacity - 1);
            let header = unsafe {
                let read_ptr = self.buffer.as_ptr().add(buffer_pos);
                match self.deserialize_header(read_ptr) {
                    Ok(h) => h,
                    Err(_) => continue, // Skip corrupted message
                }
            };

            let message_size = MessageHeader::SIZE + header.payload_length as usize;
            
            // Try to atomically reserve the message
            match self.read_pos.compare_exchange_weak(
                current_read,
                current_read.wrapping_add(message_size),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Successfully reserved, read the message
                    let message = unsafe {
                        let read_ptr = self.buffer.as_ptr().add(buffer_pos);
                        self.deserialize_message(&header, read_ptr)?
                    };
                    
                    self.stats.record_consumed();
                    return Ok(Some(message));
                }
                Err(_) => continue, // Someone else got it, try again
            }
        }
    }

    fn notify_readers(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(ref owned_fd) = self.notify_fd {
            use nix::unistd::write;
            use std::os::fd::AsRawFd;
            let fd = owned_fd.as_raw_fd();
            let value: u64 = 1;
            let buf = value.to_ne_bytes();
            let _ = write(fd, &buf); // Ignore write errors for notifications
        }
        Ok(())
    }

    unsafe fn serialize_message(&self, message: &Message, ptr: *mut u8, _size: usize) -> Result<()> {
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
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    payload_ptr,
                    data.len(),
                );
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

    unsafe fn deserialize_message(&self, header: &MessageHeader, ptr: *const u8) -> Result<Message> {
        let payload_ptr = ptr.add(MessageHeader::SIZE);
        
        let payload = if header.payload_length <= std::mem::size_of::<MessageDescriptor>() as u32 {
            let desc: MessageDescriptor = std::ptr::read_unaligned(payload_ptr as *const MessageDescriptor);
            crate::topic::MessagePayload::Descriptor(desc)
        } else {
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

    #[cfg(target_os = "linux")]
    pub fn notification_fd(&self) -> Option<RawFd> {
        use std::os::fd::AsRawFd;
        self.notify_fd.as_ref().map(|fd| fd.as_raw_fd())
    }
}

impl Drop for MPMCTopicRing {
    fn drop(&mut self) {
        // OwnedFd will automatically close on drop, no manual close needed

        let layout = std::alloc::Layout::array::<u8>(self.capacity).unwrap();
        unsafe {
            std::alloc::dealloc(self.buffer.as_ptr(), layout);
        }
    }
}

unsafe impl Send for MPMCTopicRing {}
unsafe impl Sync for MPMCTopicRing {}