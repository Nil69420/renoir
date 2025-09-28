//! Multi-producer multi-consumer ring buffer with sequence numbers

use std::{
    sync::atomic::{AtomicUsize, Ordering},
    ptr::NonNull,
    marker::PhantomData,
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
                "Capacity must be a power of 2 and greater than 0"
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

        // Initialize sequences - each slot is available for writing sequence i
        // For a capacity-4 buffer, slot 0 can hold sequences 0, 4, 8...
        // So we initialize to capacity offset before (i.e., -4 wrapped)
        unsafe {
            for i in 0..capacity {
                let init_seq = i.wrapping_sub(capacity);
                std::ptr::write(sequences.as_ptr().add(i), AtomicUsize::new(init_seq));
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

    /// Try to claim a slot for writing
    pub fn try_claim(&self) -> Result<ClaimGuard<'_, T>> {
        let seq = self.producer_seq.fetch_add(1, Ordering::Relaxed);
        let index = seq & self.mask;
        
        // Wait for the slot to be available
        let expected_seq = seq.wrapping_sub(self.capacity);
        let slot_seq = unsafe { &*self.sequences.as_ptr().add(index) };
        
        // Spin until slot is available
        while slot_seq.load(Ordering::Acquire) != expected_seq {
            if slot_seq.load(Ordering::Acquire) == expected_seq {
                break;
            }
            std::hint::spin_loop();
        }

        Ok(ClaimGuard {
            buffer: self.buffer,
            sequences: self.sequences,
            index,
            sequence: seq,
            _phantom: PhantomData,
        })
    }

    /// Try to read the next item
    pub fn try_read(&self) -> Result<T> {
        let seq = self.consumer_seq.load(Ordering::Relaxed);
        let index = seq & self.mask;
        
        let slot_seq = unsafe { &*self.sequences.as_ptr().add(index) };
        let expected_seq = seq + 1;
        
        // Check if data is available
        if slot_seq.load(Ordering::Acquire) != expected_seq {
            return Err(RenoirError::buffer_empty("SequencedRingBuffer"));
        }

        let item = unsafe { std::ptr::read(self.buffer.as_ptr().add(index)) };
        
        // Mark slot as consumed
        slot_seq.store(seq.wrapping_add(self.capacity), Ordering::Release);
        self.consumer_seq.store(seq + 1, Ordering::Release);
        
        Ok(item)
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