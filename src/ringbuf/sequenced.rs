//! Multi-producer multi-consumer ring buffer with sequence numbers

use std::{
    marker::PhantomData,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::error::{RenoirError, Result};

/// Multi-producer multi-consumer ring buffer with sequence numbers
#[derive(Debug)]
pub struct SequencedRingBuffer<T> {
    /// Underlying ring buffer
    buffer: NonNull<T>,
    /// Sequence numbers for each slot
    sequences: NonNull<AtomicUsize>,
    /// Capacity (must be power of 2)
    capacity: usize,
    /// Mask for fast modulo operation
    mask: usize,
    /// Producer sequence
    producer_seq: AtomicUsize,
    /// Consumer sequence
    consumer_seq: AtomicUsize,
    /// Phantom data for type safety
    _phantom: PhantomData<T>,
}

impl<T> SequencedRingBuffer<T> {
    /// Create a new sequenced ring buffer
    pub fn new(capacity: usize) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0",
            ));
        }

        let buffer_layout = std::alloc::Layout::array::<T>(capacity)
            .map_err(|_| RenoirError::memory("Failed to create layout for buffer"))?;

        let seq_layout = std::alloc::Layout::array::<AtomicUsize>(capacity)
            .map_err(|_| RenoirError::memory("Failed to create layout for sequences"))?;

        let buffer = unsafe {
            let ptr = std::alloc::alloc(buffer_layout) as *mut T;
            NonNull::new(ptr).ok_or_else(|| RenoirError::memory("Failed to allocate buffer"))?
        };

        let sequences = unsafe {
            let ptr = std::alloc::alloc(seq_layout) as *mut AtomicUsize;
            NonNull::new(ptr).ok_or_else(|| RenoirError::memory("Failed to allocate sequences"))?
        };

        // Initialize sequences — slot i is ready for write-sequence i.
        // Standard Disruptor initialisation: slot[i] = i.
        unsafe {
            for i in 0..capacity {
                std::ptr::write(sequences.as_ptr().add(i), AtomicUsize::new(i));
            }
        }

        Ok(Self {
            buffer,
            sequences,
            capacity,
            mask: capacity - 1,
            producer_seq: AtomicUsize::new(0),
            consumer_seq: AtomicUsize::new(0),
            _phantom: PhantomData,
        })
    }

    /// Try to claim a slot for writing (non-blocking)
    ///
    /// Returns `Err` if the buffer is full (the target slot hasn't been consumed yet).
    /// Uses CAS to avoid advancing `producer_seq` unless the slot is actually available.
    pub fn try_claim(&self) -> Result<ClaimGuard<'_, T>> {
        let mut seq = self.producer_seq.load(Ordering::Relaxed);

        loop {
            let index = seq & self.mask;
            let slot_seq = unsafe { &*self.sequences.as_ptr().add(index) };

            // Slot is available for writing when its sequence equals the
            // producer sequence we're trying to claim.
            let current = slot_seq.load(Ordering::Acquire);
            if current != seq {
                return Err(RenoirError::buffer_full("SequencedRingBuffer"));
            }

            // Try to claim this sequence number
            match self.producer_seq.compare_exchange_weak(
                seq,
                seq.wrapping_add(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Ok(ClaimGuard {
                        buffer: self.buffer,
                        sequences: self.sequences,
                        index,
                        sequence: seq,
                        _phantom: PhantomData,
                    });
                }
                Err(actual) => {
                    // Another producer claimed this seq; retry with the updated value
                    seq = actual;
                }
            }
        }
    }

    /// Try to read the next item (non-blocking)
    ///
    /// Uses CAS on `consumer_seq` so multiple consumers don't read the same slot.
    pub fn try_read(&self) -> Result<T> {
        let mut seq = self.consumer_seq.load(Ordering::Relaxed);

        loop {
            let index = seq & self.mask;
            let slot_seq = unsafe { &*self.sequences.as_ptr().add(index) };

            // Slot is readable when its sequence equals seq + 1
            // (the writer stores seq + 1 after writing).
            let current = slot_seq.load(Ordering::Acquire);
            if current != seq.wrapping_add(1) {
                return Err(RenoirError::buffer_empty("SequencedRingBuffer"));
            }

            // Try to claim this read slot
            match self.consumer_seq.compare_exchange_weak(
                seq,
                seq.wrapping_add(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let item = unsafe { std::ptr::read(self.buffer.as_ptr().add(index)) };
                    // Mark slot as consumed — available for the NEXT write at (seq + capacity)
                    slot_seq.store(seq.wrapping_add(self.capacity), Ordering::Release);
                    return Ok(item);
                }
                Err(actual) => {
                    seq = actual;
                }
            }
        }
    }

    /// Get buffer capacity
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl<T> Drop for SequencedRingBuffer<T> {
    fn drop(&mut self) {
        // Deallocate buffers
        let buffer_layout = std::alloc::Layout::array::<T>(self.capacity).unwrap();
        let seq_layout = std::alloc::Layout::array::<AtomicUsize>(self.capacity).unwrap();

        unsafe {
            std::alloc::dealloc(self.buffer.as_ptr() as *mut u8, buffer_layout);
            std::alloc::dealloc(self.sequences.as_ptr() as *mut u8, seq_layout);
        }
    }
}

unsafe impl<T: Send> Send for SequencedRingBuffer<T> {}
unsafe impl<T: Send> Sync for SequencedRingBuffer<T> {}

/// Guard for writing to a claimed slot
pub struct ClaimGuard<'a, T> {
    buffer: NonNull<T>,
    sequences: NonNull<AtomicUsize>,
    index: usize,
    sequence: usize,
    _phantom: PhantomData<&'a T>,
}

impl<'a, T> ClaimGuard<'a, T> {
    /// Write data to the claimed slot
    pub fn write(self, item: T) {
        unsafe {
            std::ptr::write(self.buffer.as_ptr().add(self.index), item);
            let slot_seq = &*self.sequences.as_ptr().add(self.index);
            slot_seq.store(self.sequence + 1, Ordering::Release);
        }
    }
}
