//! Basic lock-free single-producer single-consumer ring buffer

use std::{
    marker::PhantomData,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::error::{RenoirError, Result};

/// Lock-free single-producer single-consumer ring buffer
#[derive(Debug)]
pub struct RingBuffer<T> {
    /// Buffer storage
    buffer: NonNull<T>,
    /// Capacity (must be power of 2)
    capacity: usize,
    /// Mask for fast modulo operation
    mask: usize,
    /// Write position (producer)
    write_pos: AtomicUsize,
    /// Read position (consumer)  
    read_pos: AtomicUsize,
    /// Phantom data for type safety
    _phantom: PhantomData<T>,
}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer with the given capacity
    pub fn new(capacity: usize) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0",
            ));
        }

        let layout = std::alloc::Layout::array::<T>(capacity)
            .map_err(|_| RenoirError::memory("Failed to create layout for ring buffer"))?;

        // SAFETY: Layout is valid (non-zero, power-of-two capacity). Null check follows.
        let buffer = unsafe {
            let ptr = std::alloc::alloc(layout) as *mut T;
            NonNull::new(ptr)
                .ok_or_else(|| RenoirError::memory("Failed to allocate ring buffer"))?
        };

        Ok(Self {
            buffer,
            capacity,
            mask: capacity - 1,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            _phantom: PhantomData,
        })
    }

    /// Create a ring buffer from existing memory
    ///
    /// # Safety
    /// `memory` must point to a valid, aligned allocation of at least
    /// `capacity * size_of::<T>()` bytes that outlives the returned buffer.
    pub unsafe fn from_memory(memory: NonNull<T>, capacity: usize) -> Result<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "capacity",
                "Capacity must be a power of 2 and greater than 0",
            ));
        }

        Ok(Self {
            buffer: memory,
            capacity,
            mask: capacity - 1,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            _phantom: PhantomData,
        })
    }

    /// Get the capacity of the ring buffer
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the current number of elements in the buffer
    pub fn len(&self) -> usize {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        write_pos.wrapping_sub(read_pos)
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        write_pos == read_pos
    }

    /// Check if the buffer is full
    pub fn is_full(&self) -> bool {
        self.len() == self.capacity
    }

    /// Get available space for writing
    pub fn available_space(&self) -> usize {
        self.capacity - self.len()
    }

    /// Create a producer handle
    pub fn producer(&self) -> Producer<'_, T> {
        Producer {
            buffer: self.buffer,
            capacity: self.capacity,
            mask: self.mask,
            write_pos: &self.write_pos,
            read_pos: &self.read_pos,
            _phantom: PhantomData,
        }
    }

    /// Create a consumer handle
    pub fn consumer(&self) -> Consumer<'_, T> {
        Consumer {
            buffer: self.buffer,
            mask: self.mask,
            write_pos: &self.write_pos,
            read_pos: &self.read_pos,
            _phantom: PhantomData,
        }
    }

    /// Reset the ring buffer (clears all data)
    pub fn reset(&self) {
        self.write_pos.store(0, Ordering::Release);
        self.read_pos.store(0, Ordering::Release);
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        // Drop any remaining elements
        while !self.is_empty() {
            let read_pos = self.read_pos.load(Ordering::Acquire);
            let index = read_pos & self.mask;

            // SAFETY: Index is within capacity and slot contains a valid initialized element.
            unsafe {
                std::ptr::drop_in_place(self.buffer.as_ptr().add(index));
            }

            self.read_pos
                .store(read_pos.wrapping_add(1), Ordering::Release);
        }

        // Deallocate buffer
        let layout = std::alloc::Layout::array::<T>(self.capacity).unwrap();
        // SAFETY: Buffer was allocated with the same layout in `new` and all elements have been dropped above.
        unsafe {
            std::alloc::dealloc(self.buffer.as_ptr() as *mut u8, layout);
        }
    }
}

unsafe impl<T: Send> Send for RingBuffer<T> {}
unsafe impl<T: Send> Sync for RingBuffer<T> {}

/// Producer handle for writing to the ring buffer
#[derive(Debug)]
pub struct Producer<'a, T> {
    buffer: NonNull<T>,
    capacity: usize,
    mask: usize,
    write_pos: &'a AtomicUsize,
    read_pos: &'a AtomicUsize,
    _phantom: PhantomData<T>,
}

impl<'a, T: Clone> Producer<'a, T> {
    /// Try to push an item to the buffer
    pub fn try_push(&self, item: T) -> Result<()> {
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let read_pos = self.read_pos.load(Ordering::Acquire);

        // Check if buffer is full
        if write_pos.wrapping_sub(read_pos) >= self.capacity {
            return Err(RenoirError::buffer_full("RingBuffer"));
        }

        let index = write_pos & self.mask;

        // SAFETY: Index is within capacity (masked) and the slot is empty (checked above: buffer not full).
        unsafe {
            // Write the item
            std::ptr::write(self.buffer.as_ptr().add(index), item);
        }

        // Update write position
        self.write_pos
            .store(write_pos.wrapping_add(1), Ordering::Release);

        Ok(())
    }

    /// Push an item, blocking until space is available with exponential backoff.
    pub fn push(&self, item: T) -> Result<()> {
        let mut backoff = 0u32;
        loop {
            match self.try_push(item.clone()) {
                Ok(()) => return Ok(()),
                Err(RenoirError::BufferFull { .. }) => {
                    if backoff < 4 {
                        for _ in 0..(1 << backoff) {
                            std::hint::spin_loop();
                        }
                    } else {
                        std::thread::yield_now();
                    }
                    backoff = backoff.saturating_add(1).min(8);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Push multiple items in a batch. Returns the number of items successfully pushed.
    /// Items that don't fit are left unpushed (no partial writes per item).
    pub fn try_push_batch(&self, items: &[T]) -> usize {
        let mut pushed = 0;
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        let available = self.capacity - write_pos.wrapping_sub(read_pos);

        let count = items.len().min(available);

        // SAFETY: Each index is within capacity (masked) and the slots are empty
        // (we checked available space covers `count` items).
        unsafe {
            for (i, item) in items.iter().enumerate().take(count) {
                let index = write_pos.wrapping_add(i) & self.mask;
                std::ptr::write(self.buffer.as_ptr().add(index), item.clone());
                pushed += 1;
            }
        }

        self.write_pos
            .store(write_pos.wrapping_add(pushed), Ordering::Release);

        pushed
    }

    /// Get the current write position
    pub fn write_pos(&self) -> usize {
        self.write_pos.load(Ordering::Acquire)
    }

    /// Get available space for writing
    pub fn available_space(&self) -> usize {
        let write_pos = self.write_pos.load(Ordering::Relaxed);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        self.capacity - write_pos.wrapping_sub(read_pos)
    }
}

/// Consumer handle for reading from the ring buffer
#[derive(Debug)]
pub struct Consumer<'a, T> {
    buffer: NonNull<T>,
    mask: usize,
    write_pos: &'a AtomicUsize,
    read_pos: &'a AtomicUsize,
    _phantom: PhantomData<T>,
}

impl<'a, T> Consumer<'a, T> {
    /// Try to pop an item from the buffer
    pub fn try_pop(&self) -> Result<T> {
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let write_pos = self.write_pos.load(Ordering::Acquire);

        // Check if buffer is empty
        if read_pos == write_pos {
            return Err(RenoirError::buffer_empty("RingBuffer"));
        }

        let index = read_pos & self.mask;

        // SAFETY: Index is within capacity (masked) and the slot is initialized (checked above: buffer not empty).
        let item = unsafe {
            // Read the item
            std::ptr::read(self.buffer.as_ptr().add(index))
        };

        // Update read position
        self.read_pos
            .store(read_pos.wrapping_add(1), Ordering::Release);

        Ok(item)
    }

    /// Pop an item, blocking until one is available with exponential backoff.
    pub fn pop(&self) -> Result<T> {
        let mut backoff = 0u32;
        loop {
            match self.try_pop() {
                Ok(item) => return Ok(item),
                Err(RenoirError::BufferEmpty { .. }) => {
                    if backoff < 4 {
                        for _ in 0..(1 << backoff) {
                            std::hint::spin_loop();
                        }
                    } else {
                        std::thread::yield_now();
                    }
                    backoff = backoff.saturating_add(1).min(8);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Pop up to `max` items in a batch. Returns a Vec of popped items.
    pub fn try_pop_batch(&self, max: usize) -> Vec<T> {
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let available = write_pos.wrapping_sub(read_pos);

        let count = max.min(available);
        let mut items = Vec::with_capacity(count);

        // SAFETY: Each index is within capacity (masked) and the slots are initialized
        // (we checked available items covers `count`).
        unsafe {
            for i in 0..count {
                let index = read_pos.wrapping_add(i) & self.mask;
                items.push(std::ptr::read(self.buffer.as_ptr().add(index)));
            }
        }

        self.read_pos
            .store(read_pos.wrapping_add(count), Ordering::Release);

        items
    }

    /// Peek at the next item without consuming it
    pub fn peek(&self) -> Result<&T> {
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        let write_pos = self.write_pos.load(Ordering::Acquire);

        // Check if buffer is empty
        if read_pos == write_pos {
            return Err(RenoirError::buffer_empty("RingBuffer"));
        }

        let index = read_pos & self.mask;

        // SAFETY: Index is within capacity (masked) and the slot is initialized (buffer not empty).
        Ok(unsafe { &*self.buffer.as_ptr().add(index) })
    }

    /// Get the current read position
    pub fn read_pos(&self) -> usize {
        self.read_pos.load(Ordering::Acquire)
    }

    /// Get the number of available items
    pub fn available_items(&self) -> usize {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Relaxed);
        write_pos.wrapping_sub(read_pos)
    }
}
