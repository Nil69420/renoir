//! Multi Producer Multi Consumer ring buffer for topic messaging

use std::{
    os::fd::RawFd,
    ptr::NonNull,
    sync::{
        atomic::{AtomicU8, AtomicUsize, Ordering},
        Arc,
    },
};

use crate::{
    error::{RenoirError, Result},
    sync::MioEventNotification,
    topic::{Message, MessageDescriptor, MessageHeader, TopicStats},
};

/// Maximum wait iterations for a producer publishing after an earlier producer
const MAX_PUBLISH_WAIT: usize = 1_000_000;

/// Magic number identifying an MPMC ring control block in shared memory
pub const MPMC_RING_MAGIC: u64 = 0x4D50_4D43_524E_4F52;

pub use crate::topic_rings::spsc::{
    RingControlInfo, SESSION_CLEAN, SESSION_DIRTY, SESSION_MARKER_OFFSET,
};

/// Control block for an MPMC ring, placed at the start of its memory region.
/// Lives in shared memory when the ring is created over a shared region so
/// that all attaching processes observe the same positions.
#[repr(C)]
#[derive(Debug)]
struct MPMCRingHeader {
    magic: u64,
    capacity: usize,
    session: AtomicU8,
    write_pos: AtomicUsize,
    published_pos: AtomicUsize,
    read_pos: AtomicUsize,
}

// The session marker must sit at the shared offset so the region-open code
// can read it without knowing the ring type
const _: () = assert!(std::mem::offset_of!(MPMCRingHeader, session) == SESSION_MARKER_OFFSET);

/// Multi-Producer Multi-Consumer ring buffer for topics with multiple writers/readers
#[derive(Debug)]
pub struct MPMCTopicRing {
    /// Control block (positions) in the ring's memory region
    header: NonNull<MPMCRingHeader>,
    /// Buffer storage
    buffer: NonNull<u8>,
    /// Buffer capacity in bytes
    capacity: usize,
    /// Whether this ring owns its memory (heap-allocated) or borrows it
    owns_memory: bool,
    /// Topic statistics
    stats: Arc<TopicStats>,
    /// Mio-based event notification system
    notifier: Arc<MioEventNotification>,
}

impl MPMCTopicRing {
    /// Total memory needed for a ring with the given message buffer capacity
    pub fn required_memory_size(capacity: usize) -> usize {
        std::mem::size_of::<MPMCRingHeader>() + capacity
    }

    /// Create a new MPMC topic ring
    pub fn new(capacity: usize, stats: Arc<TopicStats>) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0",
            ));
        }

        let total = Self::required_memory_size(capacity);
        let layout =
            std::alloc::Layout::from_size_align(total, std::mem::align_of::<MPMCRingHeader>())
                .map_err(|_| RenoirError::memory("Failed to create layout for MPMC ring"))?;

        let base = unsafe {
            let ptr = std::alloc::alloc_zeroed(layout);
            NonNull::new(ptr).ok_or_else(|| RenoirError::memory("Failed to allocate MPMC ring"))?
        };

        unsafe {
            base.cast::<MPMCRingHeader>()
                .as_ptr()
                .write(MPMCRingHeader {
                    magic: MPMC_RING_MAGIC,
                    capacity,
                    session: AtomicU8::new(SESSION_DIRTY),
                    write_pos: AtomicUsize::new(0),
                    published_pos: AtomicUsize::new(0),
                    read_pos: AtomicUsize::new(0),
                });
        }

        let notifier = MioEventNotification::new()
            .map_err(|_| RenoirError::memory("Failed to create event notifier"))?;

        Ok(Self {
            header: base.cast::<MPMCRingHeader>(),
            buffer: unsafe {
                NonNull::new_unchecked(base.as_ptr().add(std::mem::size_of::<MPMCRingHeader>()))
            },
            capacity,
            owns_memory: true,
            stats,
            notifier,
        })
    }

    /// Create a ring over caller-provided memory.
    ///
    /// If the memory does not yet contain a ring control block (magic
    /// mismatch), the control block is initialized and the caller takes the
    /// creator role. If it already contains one, this attaches to the existing
    /// ring, which must have the same capacity.
    ///
    /// # Safety
    /// `memory` must point to a valid, aligned allocation of at least
    /// `memory_size` bytes, where `memory_size >= required_memory_size(capacity)`.
    /// The memory must remain valid for the lifetime of this struct and must
    /// not be initialized concurrently by multiple creators.
    pub unsafe fn from_memory(
        memory: NonNull<u8>,
        memory_size: usize,
        capacity: usize,
        stats: Arc<TopicStats>,
    ) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0",
            ));
        }

        let required = Self::required_memory_size(capacity);
        if memory_size < required {
            return Err(RenoirError::insufficient_space(required, memory_size));
        }

        let alignment = std::mem::align_of::<MPMCRingHeader>();
        if !(memory.as_ptr() as usize).is_multiple_of(alignment) {
            return Err(RenoirError::alignment(memory.as_ptr() as usize, alignment));
        }

        let header_ptr = memory.cast::<MPMCRingHeader>();
        let existing_magic = unsafe { (*header_ptr.as_ptr()).magic };
        let existing_capacity = unsafe { (*header_ptr.as_ptr()).capacity };

        if existing_magic == MPMC_RING_MAGIC {
            if existing_capacity != capacity {
                return Err(RenoirError::invalid_parameter(
                    "capacity",
                    "Existing ring has a different capacity",
                ));
            }
        } else {
            unsafe {
                header_ptr.as_ptr().write(MPMCRingHeader {
                    magic: MPMC_RING_MAGIC,
                    capacity,
                    session: AtomicU8::new(SESSION_DIRTY),
                    write_pos: AtomicUsize::new(0),
                    published_pos: AtomicUsize::new(0),
                    read_pos: AtomicUsize::new(0),
                });
            }
        }

        let notifier = MioEventNotification::new()
            .map_err(|_| RenoirError::memory("Failed to create event notifier"))?;

        Ok(Self {
            header: header_ptr,
            buffer: unsafe {
                NonNull::new_unchecked(memory.as_ptr().add(std::mem::size_of::<MPMCRingHeader>()))
            },
            capacity,
            owns_memory: false,
            stats,
            notifier,
        })
    }

    /// Get the buffer capacity
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Read a snapshot of the ring's shared control block from raw region
    /// memory (e.g. another process's shared region), without constructing
    /// a ring. Returns None if the memory doesn't contain a valid MPMC ring.
    pub fn read_control_info(memory: &[u8]) -> Option<RingControlInfo> {
        use super::header_bytes::{read_u64, read_usize, valid_capacity};
        if memory.len() < std::mem::size_of::<MPMCRingHeader>() {
            return None;
        }
        if read_u64(memory, std::mem::offset_of!(MPMCRingHeader, magic))? != MPMC_RING_MAGIC {
            return None;
        }
        let capacity = read_usize(memory, std::mem::offset_of!(MPMCRingHeader, capacity))?;
        if !valid_capacity(capacity, memory.len()) {
            return None;
        }
        Some(RingControlInfo {
            capacity,
            write_pos: read_usize(memory, std::mem::offset_of!(MPMCRingHeader, write_pos))?,
            read_pos: read_usize(memory, std::mem::offset_of!(MPMCRingHeader, read_pos))?,
        })
    }

    /// Read the header of the next pending message from raw region memory,
    /// returning (sequence, publish timestamp ns). Returns None if the ring
    /// is empty or the memory doesn't hold a valid MPMC ring.
    pub fn read_pending_header(memory: &[u8]) -> Option<(u64, u64)> {
        let info = Self::read_control_info(memory)?;
        let pending = info.pending_bytes();
        if pending < MessageHeader::SIZE {
            return None;
        }

        if info.capacity < MessageHeader::SIZE {
            return None;
        }
        let header_start = std::mem::size_of::<MPMCRingHeader>();
        let start = info.read_pos & (info.capacity - 1);
        let first = usize::min(MessageHeader::SIZE, info.capacity - start);

        let mut header_bytes = [0u8; MessageHeader::SIZE];
        if memory.len() < header_start + info.capacity {
            return None;
        }
        let buffer = &memory[header_start..header_start + info.capacity];
        header_bytes[..first].copy_from_slice(&buffer[start..start + first]);
        if MessageHeader::SIZE > first {
            header_bytes[first..].copy_from_slice(&buffer[..MessageHeader::SIZE - first]);
        }

        let header: MessageHeader =
            unsafe { std::ptr::read_unaligned(header_bytes.as_ptr() as *const MessageHeader) };
        if header.validate().is_err() {
            return None;
        }
        Some((header.sequence, header.timestamp))
    }

    /// Try to publish a message (thread-safe for multiple producers)
    pub fn try_publish(&self, message: &Message) -> Result<()> {
        let message_size = message.total_size();

        // Atomic allocation of write space
        let write_start = loop {
            let current_write = self.write_pos().load(Ordering::Relaxed);
            let current_read = self.read_pos().load(Ordering::Acquire);

            // Check if we have enough space
            if current_write.wrapping_sub(current_read) + message_size > self.capacity {
                self.stats.record_dropped();
                return Err(RenoirError::buffer_full("MPMC ring is full"));
            }

            // Try to atomically reserve space
            match self.write_pos().compare_exchange_weak(
                current_write,
                current_write.wrapping_add(message_size),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break current_write,
                Err(_) => continue, // Retry on contention
            }
        };

        // Serialize and write the message (before publishing the position)
        unsafe {
            self.serialize_message(message, write_start)?;
        }

        // Publish the reserved region in order so consumers only ever observe
        // fully-serialized messages
        let write_end = write_start.wrapping_add(message_size);
        let mut wait_count = 0usize;
        loop {
            let published = self.published_pos().load(Ordering::Acquire);
            if published == write_start {
                self.published_pos().store(write_end, Ordering::Release);
                break;
            }
            wait_count += 1;
            if wait_count > MAX_PUBLISH_WAIT {
                return Err(RenoirError::concurrency(
                    "MPMC publish timed out waiting for a prior producer",
                ));
            }
            std::thread::yield_now();
        }

        // Record statistics
        self.stats.record_published_stats(message_size);

        // Notify readers if enabled
        if self.notifier.is_enabled() {
            let _ = self.notifier.notify(); // Ignore notification errors
        }

        Ok(())
    }

    /// Try to consume a message (thread-safe for multiple consumers)
    pub fn try_consume(&self) -> Result<Option<Message>> {
        loop {
            let current_read = self.read_pos().load(Ordering::Relaxed);
            let current_write = self.published_pos().load(Ordering::Acquire);

            if current_read == current_write {
                return Ok(None); // No data available
            }

            // Read message header
            let header = unsafe { self.deserialize_header(current_read)? };

            let message_size = MessageHeader::SIZE + header.payload_length as usize;

            // Ensure the message doesn't extend past the published region
            if current_write.wrapping_sub(current_read) < message_size {
                return Err(RenoirError::serialization(
                    "MPMC message size exceeds published data",
                ));
            }

            // Try to atomically reserve the message
            match self.read_pos().compare_exchange_weak(
                current_read,
                current_read.wrapping_add(message_size),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Successfully reserved, read the message
                    let message = unsafe { self.deserialize_message(&header, current_read)? };

                    let latency_ns = now_ns().saturating_sub(message.header.timestamp);
                    self.stats.record_consumed(latency_ns);
                    return Ok(Some(message));
                }
                Err(_) => continue, // Someone else got it, try again
            }
        }
    }

    /// Peek at the next message without consuming it
    /// Returns a copy of the message at the head of the queue without advancing the read position
    /// Note: In MPMC mode, the message may be consumed by another thread between peek and consume
    pub fn try_peek(&self) -> Result<Option<Message>> {
        let current_read = self.read_pos().load(Ordering::Acquire);
        let current_write = self.published_pos().load(Ordering::Acquire);

        if current_read == current_write {
            return Ok(None); // No data available
        }

        // Read message header
        let header = unsafe { self.deserialize_header(current_read)? };

        let message_size = MessageHeader::SIZE + header.payload_length as usize;

        // Check if we have enough data for the complete message
        if current_write.wrapping_sub(current_read) < message_size {
            return Ok(None); // Incomplete message
        }

        // Read the complete message (but don't advance read position)
        let message = unsafe { self.deserialize_message(&header, current_read)? };

        // Note: read_pos is NOT updated - this is the key difference from try_consume
        // In MPMC, another consumer may consume this message before the caller can act on it
        Ok(Some(message))
    }

    unsafe fn copy_into_ring(&self, abs_pos: usize, src: *const u8, len: usize) {
        let start = abs_pos & (self.capacity - 1);
        let first = usize::min(len, self.capacity - start);

        std::ptr::copy_nonoverlapping(src, self.buffer.as_ptr().add(start), first);
        if len > first {
            std::ptr::copy_nonoverlapping(src.add(first), self.buffer.as_ptr(), len - first);
        }
    }

    unsafe fn copy_from_ring(&self, abs_pos: usize, dst: *mut u8, len: usize) {
        let start = abs_pos & (self.capacity - 1);
        let first = usize::min(len, self.capacity - start);

        std::ptr::copy_nonoverlapping(self.buffer.as_ptr().add(start), dst, first);
        if len > first {
            std::ptr::copy_nonoverlapping(self.buffer.as_ptr(), dst.add(first), len - first);
        }
    }

    unsafe fn serialize_message(&self, message: &Message, write_pos: usize) -> Result<()> {
        // Write header
        self.copy_into_ring(
            write_pos,
            &message.header as *const MessageHeader as *const u8,
            MessageHeader::SIZE,
        );

        // Write payload
        let payload_pos = write_pos.wrapping_add(MessageHeader::SIZE);
        match &message.payload {
            crate::topic::MessagePayload::Inline(data) => {
                self.copy_into_ring(payload_pos, data.as_ptr(), data.len());
            }
            crate::topic::MessagePayload::Descriptor(desc) => {
                self.copy_into_ring(
                    payload_pos,
                    desc as *const MessageDescriptor as *const u8,
                    std::mem::size_of::<MessageDescriptor>(),
                );
            }
        }

        Ok(())
    }

    unsafe fn deserialize_header(&self, read_pos: usize) -> Result<MessageHeader> {
        let mut header_bytes = [0u8; MessageHeader::SIZE];
        self.copy_from_ring(read_pos, header_bytes.as_mut_ptr(), MessageHeader::SIZE);
        let header: MessageHeader = std::ptr::read_unaligned(header_bytes.as_ptr() as *const _);
        header.validate()?;
        Ok(header)
    }

    unsafe fn deserialize_message(
        &self,
        header: &MessageHeader,
        read_pos: usize,
    ) -> Result<Message> {
        let payload_pos = read_pos.wrapping_add(MessageHeader::SIZE);

        let payload = match header.format_type() {
            crate::topic::FORMAT_TYPE_DESCRIPTOR => {
                let mut desc_bytes = [0u8; std::mem::size_of::<MessageDescriptor>()];
                self.copy_from_ring(payload_pos, desc_bytes.as_mut_ptr(), desc_bytes.len());
                let desc: MessageDescriptor =
                    std::ptr::read_unaligned(desc_bytes.as_ptr() as *const MessageDescriptor);
                crate::topic::MessagePayload::Descriptor(desc)
            }
            crate::topic::FORMAT_TYPE_INLINE => {
                let mut data = vec![0u8; header.payload_length as usize];
                self.copy_from_ring(payload_pos, data.as_mut_ptr(), data.len());
                crate::topic::MessagePayload::Inline(data)
            }
            _ => {
                return Err(RenoirError::serialization("Unknown message format type"));
            }
        };

        Ok(Message {
            header: *header,
            payload,
        })
    }

    fn header(&self) -> &MPMCRingHeader {
        // SAFETY: header always points to a valid control block for the
        // lifetime of self (owned allocation or borrowed caller memory)
        unsafe { self.header.as_ref() }
    }

    fn write_pos(&self) -> &AtomicUsize {
        &self.header().write_pos
    }

    fn published_pos(&self) -> &AtomicUsize {
        &self.header().published_pos
    }

    fn read_pos(&self) -> &AtomicUsize {
        &self.header().read_pos
    }

    /// Get the eventfd for external polling
    #[cfg(target_os = "linux")]
    pub fn notification_fd(&self) -> Option<RawFd> {
        self.notifier.event_fd()
    }
}

impl Drop for MPMCTopicRing {
    fn drop(&mut self) {
        // MioEventNotification will automatically clean up eventfd on drop

        // Deallocate buffer only if we own it (from_memory borrows)
        if self.owns_memory {
            let total = Self::required_memory_size(self.capacity);
            let layout =
                std::alloc::Layout::from_size_align(total, std::mem::align_of::<MPMCRingHeader>())
                    .unwrap();
            unsafe {
                std::alloc::dealloc(self.header.as_ptr() as *mut u8, layout);
            }
        }
    }
}

unsafe impl Send for MPMCTopicRing {}
unsafe impl Sync for MPMCTopicRing {}

/// Current wall-clock time in nanoseconds since the UNIX epoch
fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
