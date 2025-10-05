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

            unsafe {
                std::ptr::drop_in_place(self.buffer.as_ptr().add(index));
            }

            self.read_pos
                .store(read_pos.wrapping_add(1), Ordering::Release);
        }

        // Deallocate buffer
        let layout = std::alloc::Layout::array::<T>(self.capacity).unwrap();
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

        unsafe {
            // Write the item
            std::ptr::write(self.buffer.as_ptr().add(index), item);
        }

        // Update write position
        self.write_pos
            .store(write_pos.wrapping_add(1), Ordering::Release);

        Ok(())
    }

    /// Push an item, blocking until space is available (spinning)
    pub fn push(&self, item: T) -> Result<()> {
        loop {
            match self.try_push(item.clone()) {
                Ok(()) => return Ok(()),
                Err(RenoirError::BufferFull { .. }) => {
                    // Spin-wait for space
                    std::hint::spin_loop();
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
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

        let item = unsafe {
            // Read the item
            std::ptr::read(self.buffer.as_ptr().add(index))
        };

        // Update read position
        self.read_pos
            .store(read_pos.wrapping_add(1), Ordering::Release);

        Ok(item)
    }

    /// Pop an item, blocking until one is available (spinning)
    pub fn pop(&self) -> Result<T> {
        loop {
            match self.try_pop() {
                Ok(item) => return Ok(item),
                Err(RenoirError::BufferEmpty { .. }) => {
                    // Spin-wait for data
                    std::hint::spin_loop();
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
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
