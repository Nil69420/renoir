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

/// Magic number identifying an SPSC ring control block in shared memory
pub const SPSC_RING_MAGIC: u64 = 0x5350_5343_524E_4F52;

/// Byte offset of the session marker within the ring control block. Both
/// ring types place `magic` at 0 and `capacity` at 8, so the marker lands
/// at 16 in both, letting the region-open code decide wipe-vs-preserve
/// without knowing the ring type.
pub const SESSION_MARKER_OFFSET: usize = 16;

/// Session marker value: previous holder exited cleanly, content preserved
pub const SESSION_CLEAN: u8 = 0;

/// Session marker value: previous holder crashed mid-write, content wiped
pub const SESSION_DIRTY: u8 = 1;

/// Read-only snapshot of a ring's shared control block (cross-process
/// introspection). `pending_bytes` is write_pos - read_pos; `utilization`
/// is pending_bytes / capacity.
#[derive(Debug, Clone, Copy)]
pub struct RingControlInfo {
    /// Ring buffer capacity in bytes
    pub capacity: usize,
    /// Absolute write position
    pub write_pos: usize,
    /// Absolute read position
    pub read_pos: usize,
}

impl RingControlInfo {
    /// Number of bytes currently pending in the ring
    pub fn pending_bytes(&self) -> usize {
        self.write_pos.wrapping_sub(self.read_pos)
    }

    /// Utilization in 0.0..=1.0
    pub fn utilization(&self) -> f64 {
        if self.capacity == 0 {
            return 0.0;
        }
        self.pending_bytes() as f64 / self.capacity as f64
    }
}

/// Control block for an SPSC ring, placed at the start of its memory region.
/// Lives in shared memory when the ring is created over a shared region so
/// that all attaching processes observe the same positions.
#[repr(C)]
#[derive(Debug)]
struct SPSCRingHeader {
    magic: u64,
    capacity: usize,
    session: u8,
    write_pos: AtomicUsize,
    read_pos: AtomicUsize,
    cached_read_pos: AtomicUsize,
    cached_write_pos: AtomicUsize,
}

// The session marker must sit at the shared offset so the region-open code
// can read it without knowing the ring type
const _: () = assert!(std::mem::offset_of!(SPSCRingHeader, session) == SESSION_MARKER_OFFSET);

/// Single Producer Single Consumer ring buffer optimized for topic messaging
#[derive(Debug)]
pub struct SPSCTopicRing {
    /// Control block (positions) in the ring's memory region
    header: NonNull<SPSCRingHeader>,
    /// Buffer storage for messages
    buffer: NonNull<u8>,
    /// Total buffer capacity in bytes
    capacity: usize,
    /// Whether this ring owns its memory (heap-allocated) or borrows it
    owns_memory: bool,
    /// Topic statistics
    stats: Arc<TopicStats>,
    /// Mio-based event notification system
    notifier: Arc<MioEventNotification>,
}

impl SPSCTopicRing {
    /// Total memory needed for a ring with the given message buffer capacity
    pub fn required_memory_size(capacity: usize) -> usize {
        std::mem::size_of::<SPSCRingHeader>() + capacity
    }

    /// Create a new SPSC topic ring
    pub fn new(capacity: usize, stats: Arc<TopicStats>) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0",
            ));
        }

        let total = Self::required_memory_size(capacity);
        let layout =
            std::alloc::Layout::from_size_align(total, std::mem::align_of::<SPSCRingHeader>())
                .map_err(|_| RenoirError::memory("Failed to create layout for topic ring"))?;

        let base = unsafe {
            let ptr = std::alloc::alloc_zeroed(layout);
            NonNull::new(ptr).ok_or_else(|| RenoirError::memory("Failed to allocate topic ring"))?
        };

        unsafe {
            base.cast::<SPSCRingHeader>()
                .as_ptr()
                .write(SPSCRingHeader {
                    magic: SPSC_RING_MAGIC,
                    capacity,
                    session: SESSION_DIRTY,
                    write_pos: AtomicUsize::new(0),
                    read_pos: AtomicUsize::new(0),
                    cached_read_pos: AtomicUsize::new(0),
                    cached_write_pos: AtomicUsize::new(0),
                });
        }

        let notifier = MioEventNotification::new()
            .map_err(|_| RenoirError::memory("Failed to create event notifier"))?;

        Ok(Self {
            header: base.cast::<SPSCRingHeader>(),
            buffer: unsafe {
                NonNull::new_unchecked(base.as_ptr().add(std::mem::size_of::<SPSCRingHeader>()))
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

        let alignment = std::mem::align_of::<SPSCRingHeader>();
        if !(memory.as_ptr() as usize).is_multiple_of(alignment) {
            return Err(RenoirError::alignment(memory.as_ptr() as usize, alignment));
        }

        let header_ptr = memory.cast::<SPSCRingHeader>();
        let existing_magic = unsafe { (*header_ptr.as_ptr()).magic };
        let existing_capacity = unsafe { (*header_ptr.as_ptr()).capacity };

        if existing_magic == SPSC_RING_MAGIC {
            if existing_capacity != capacity {
                return Err(RenoirError::invalid_parameter(
                    "capacity",
                    "Existing ring has a different capacity",
                ));
            }
        } else {
            unsafe {
                header_ptr.as_ptr().write(SPSCRingHeader {
                    magic: SPSC_RING_MAGIC,
                    capacity,
                    session: SESSION_DIRTY,
                    write_pos: AtomicUsize::new(0),
                    read_pos: AtomicUsize::new(0),
                    cached_read_pos: AtomicUsize::new(0),
                    cached_write_pos: AtomicUsize::new(0),
                });
            }
        }

        let notifier = MioEventNotification::new()
            .map_err(|_| RenoirError::memory("Failed to create event notifier"))?;

        Ok(Self {
            header: header_ptr,
            buffer: unsafe {
                NonNull::new_unchecked(memory.as_ptr().add(std::mem::size_of::<SPSCRingHeader>()))
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
    /// a ring. Returns None if the memory doesn't contain a valid SPSC ring.
    pub fn read_control_info(memory: &[u8]) -> Option<RingControlInfo> {
        if memory.len() < std::mem::size_of::<SPSCRingHeader>() {
            return None;
        }
        let header = unsafe { &*(memory.as_ptr() as *const SPSCRingHeader) };
        if header.magic != SPSC_RING_MAGIC {
            return None;
        }
        Some(RingControlInfo {
            capacity: header.capacity,
            write_pos: header.write_pos.load(Ordering::Relaxed),
            read_pos: header.read_pos.load(Ordering::Relaxed),
        })
    }

    /// Read the header of the next pending message from raw region memory,
    /// returning (sequence, publish timestamp ns). Returns None if the ring
    /// is empty or the memory doesn't hold a valid SPSC ring.
    pub fn read_pending_header(memory: &[u8]) -> Option<(u64, u64)> {
        let info = Self::read_control_info(memory)?;
        let pending = info.pending_bytes();
        if pending < MessageHeader::SIZE {
            return None;
        }

        let header_start = std::mem::size_of::<SPSCRingHeader>();
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

    /// Get available space for writing (bytes)
    pub fn available_write_space(&self) -> usize {
        let write_pos = self.write_pos().load(Ordering::Relaxed);
        let fresh_read = self.read_pos().load(Ordering::Acquire);
        self.cached_read_pos().store(fresh_read, Ordering::Relaxed);

        let used = write_pos.wrapping_sub(fresh_read);
        self.capacity.saturating_sub(used)
    }

    /// Try to publish a message (producer side)
    pub fn try_publish(&self, message: &Message) -> Result<()> {
        let message_size = message.total_size();
        let avail = self.available_write_space();

        // Check if we have enough space
        if avail < message_size {
            self.stats.record_dropped();
            return Err(RenoirError::buffer_full("Topic ring is full"));
        }

        // Serialize and write the message
        let write_pos = self.write_pos().load(Ordering::Relaxed);
        unsafe {
            self.serialize_message(message, write_pos)?;
        }

        // Update write position
        let new_write_pos = write_pos.wrapping_add(message_size);
        self.write_pos().store(new_write_pos, Ordering::Release);

        // Record statistics
        self.stats.record_published_stats(message_size);

        // Notify readers if enabled
        if self.notifier.is_enabled() {
            let _ = self.notifier.notify(); // Ignore notification errors
        }

        Ok(())
    }

    /// Try to consume a message (consumer side)
    pub fn try_consume(&self) -> Result<Option<Message>> {
        let read_pos = self.read_pos().load(Ordering::Relaxed);
        let cached_write = self.cached_write_pos().load(Ordering::Relaxed);
        let mut available = cached_write.wrapping_sub(read_pos);

        if available < MessageHeader::SIZE {
            let fresh_write = self.write_pos().load(Ordering::Acquire);
            self.cached_write_pos()
                .store(fresh_write, Ordering::Relaxed);
            available = fresh_write.wrapping_sub(read_pos);

            if available < MessageHeader::SIZE {
                return Ok(None);
            }
        }

        let header = unsafe { self.deserialize_header(read_pos)? };

        let message_size = MessageHeader::SIZE + header.payload_length as usize;

        if available < message_size {
            let fresh_write = self.write_pos().load(Ordering::Acquire);
            self.cached_write_pos()
                .store(fresh_write, Ordering::Relaxed);
            available = fresh_write.wrapping_sub(read_pos);
            if available < message_size {
                return Ok(None);
            }
        }

        let message = unsafe { self.deserialize_message(&header, read_pos)? };

        // Update read position
        self.read_pos()
            .store(read_pos.wrapping_add(message_size), Ordering::Release);

        // Record statistics (end-to-end latency: publish -> consume)
        let latency_ns = now_ns().saturating_sub(message.header.timestamp);
        self.stats.record_consumed(latency_ns);

        Ok(Some(message))
    }

    /// Peek at the next message without consuming it
    /// Returns a copy of the message at the head of the queue without advancing the read position
    pub fn try_peek(&self) -> Result<Option<Message>> {
        let read_pos = self.read_pos().load(Ordering::Relaxed);
        let cached_write = self.cached_write_pos().load(Ordering::Relaxed);
        let mut available = cached_write.wrapping_sub(read_pos);

        if available < MessageHeader::SIZE {
            let fresh_write = self.write_pos().load(Ordering::Acquire);
            self.cached_write_pos()
                .store(fresh_write, Ordering::Relaxed);
            available = fresh_write.wrapping_sub(read_pos);

            if available < MessageHeader::SIZE {
                return Ok(None);
            }
        }

        let header = unsafe { self.deserialize_header(read_pos)? };

        let message_size = MessageHeader::SIZE + header.payload_length as usize;

        if available < message_size {
            let fresh_write = self.write_pos().load(Ordering::Acquire);
            self.cached_write_pos()
                .store(fresh_write, Ordering::Relaxed);
            available = fresh_write.wrapping_sub(read_pos);
            if available < message_size {
                return Ok(None);
            }
        }

        let message = unsafe { self.deserialize_message(&header, read_pos)? };

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
        self.copy_into_ring(
            write_pos,
            &message.header as *const MessageHeader as *const u8,
            MessageHeader::SIZE,
        );

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

    fn header(&self) -> &SPSCRingHeader {
        // SAFETY: header always points to a valid control block for the
        // lifetime of self (owned allocation or borrowed caller memory)
        unsafe { self.header.as_ref() }
    }

    fn write_pos(&self) -> &AtomicUsize {
        &self.header().write_pos
    }

    fn read_pos(&self) -> &AtomicUsize {
        &self.header().read_pos
    }

    fn cached_read_pos(&self) -> &AtomicUsize {
        &self.header().cached_read_pos
    }

    fn cached_write_pos(&self) -> &AtomicUsize {
        &self.header().cached_write_pos
    }

    /// Get the eventfd for external polling
    #[cfg(target_os = "linux")]
    pub fn notification_fd(&self) -> Option<RawFd> {
        self.notifier.event_fd()
    }

    /// Get current buffer utilization (0.0 to 1.0)
    pub fn utilization(&self) -> f32 {
        let write_pos = self.write_pos().load(Ordering::Relaxed);
        let read_pos = self.read_pos().load(Ordering::Relaxed);
        let used = write_pos.wrapping_sub(read_pos);
        used as f32 / self.capacity as f32
    }
}

impl Drop for SPSCTopicRing {
    fn drop(&mut self) {
        // MioEventNotification will automatically clean up eventfd on drop

        // Deallocate buffer only if we own it (from_memory borrows)
        if self.owns_memory {
            let total = Self::required_memory_size(self.capacity);
            let layout =
                std::alloc::Layout::from_size_align(total, std::mem::align_of::<SPSCRingHeader>())
                    .unwrap();
            unsafe {
                std::alloc::dealloc(self.header.as_ptr() as *mut u8, layout);
            }
        }
    }
}

unsafe impl Send for SPSCTopicRing {}
unsafe impl Sync for SPSCTopicRing {}

/// Current wall-clock time in nanoseconds since the UNIX epoch
fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
