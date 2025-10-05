//! Single Writer Multiple Reader (SWMR) ring buffer
//!
//! This module implements the SWMR pattern optimized for robotics scenarios where
//! a single sensor/data source needs to efficiently broadcast to multiple consumers.
//!
//! Key features:
//! - Lock-free writer path with atomic sequence updates
//! - Lock-free reader paths with sequence consistency checking  
//! - Memory barriers ensure visibility across threads/processes
//! - Slot-based design allows readers to proceed at different rates
//! - No reader synchronization needed (each reader tracks its own position)

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use super::{
    sequence::{special, AtomicSequence, SequenceChecker, SequenceNumber},
    SyncError, SyncResult,
};

/// A slot in the SWMR ring buffer
#[repr(C)]
pub struct SWMRSlot<T> {
    /// Sequence number for this slot
    sequence: AtomicSequence,
    /// Data payload (accessed after sequence check)
    data: UnsafeCell<MaybeUninit<T>>,
}

impl<T> SWMRSlot<T> {
    /// Create a new empty slot
    fn new() -> Self {
        Self {
            sequence: AtomicSequence::new(),
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Get the sequence for this slot
    fn sequence(&self) -> &AtomicSequence {
        &self.sequence
    }

    /// Get raw data pointer (unsafe, caller must ensure sequence consistency)
    unsafe fn data_ptr(&self) -> *mut T {
        (*self.data.get()).as_mut_ptr()
    }
}

unsafe impl<T: Send> Send for SWMRSlot<T> {}
unsafe impl<T: Send + Sync> Sync for SWMRSlot<T> {}

/// SWMR ring buffer for efficient single-writer, multiple-reader scenarios
#[derive(Debug)]
pub struct SWMRRing<T> {
    /// Ring buffer slots
    slots: NonNull<SWMRSlot<T>>,
    /// Ring buffer capacity (power of 2)
    capacity: usize,
    /// Mask for fast modulo (capacity - 1)
    mask: usize,
    /// Current write position
    write_pos: AtomicUsize,
    /// Global sequence generator
    writer_sequence: AtomicUsize,
    /// Phantom data for type safety
    _phantom: PhantomData<T>,
}

impl<T> SWMRRing<T> {
    /// Create a new SWMR ring buffer
    pub fn new(capacity: usize) -> SyncResult<Self> {
        if capacity == 0 || !capacity.is_power_of_two() {
            return Err(SyncError::NoSlotsAvailable);
        }

        let layout = std::alloc::Layout::array::<SWMRSlot<T>>(capacity)
            .map_err(|_| SyncError::CorruptedBuffer)?;

        let slots = unsafe {
            let ptr = std::alloc::alloc(layout) as *mut SWMRSlot<T>;
            if ptr.is_null() {
                return Err(SyncError::CorruptedBuffer);
            }

            // Initialize all slots
            for i in 0..capacity {
                ptr.add(i).write(SWMRSlot::new());
            }

            NonNull::new_unchecked(ptr)
        };

        Ok(Self {
            slots,
            capacity,
            mask: capacity - 1,
            write_pos: AtomicUsize::new(0),
            writer_sequence: AtomicUsize::new(special::FIRST_DATA as usize),
            _phantom: PhantomData,
        })
    }

    /// Get total capacity of the ring
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Create a writer handle for this ring
    pub fn writer(&self) -> WriterHandle<'_, T> {
        WriterHandle { ring: self }
    }

    /// Create a reader handle for this ring
    pub fn reader(&self) -> ReaderHandle<'_, T> {
        ReaderHandle {
            ring: self,
            read_pos: 0,
            last_sequence: special::EMPTY,
        }
    }

    /// Get slot at given index
    fn slot(&self, index: usize) -> &SWMRSlot<T> {
        unsafe { &*self.slots.as_ptr().add(index & self.mask) }
    }

    /// Get next sequence number for writer
    fn next_writer_sequence(&self) -> SequenceNumber {
        self.writer_sequence.fetch_add(1, Ordering::Relaxed) as SequenceNumber
    }
}

impl<T> Drop for SWMRRing<T> {
    fn drop(&mut self) {
        unsafe {
            // Drop all initialized slots
            for i in 0..self.capacity {
                let slot = &*self.slots.as_ptr().add(i);
                if AtomicSequence::is_ready(slot.sequence.load_relaxed()) {
                    std::ptr::drop_in_place(slot.data_ptr());
                }
            }

            let layout = std::alloc::Layout::array::<SWMRSlot<T>>(self.capacity).unwrap();
            std::alloc::dealloc(self.slots.as_ptr() as *mut u8, layout);
        }
    }
}

unsafe impl<T: Send> Send for SWMRRing<T> {}
unsafe impl<T: Send + Sync> Sync for SWMRRing<T> {}

/// Writer handle for SWMR ring buffer
///
/// Only one writer should exist per ring. The writer is responsible for
/// updating slots with new data and managing the write sequence.
pub struct WriterHandle<'a, T> {
    ring: &'a SWMRRing<T>,
}

impl<'a, T> WriterHandle<'a, T> {
    /// Try to write data to the next available slot
    ///
    /// This is the hot path - optimized to be lock-free and fast
    pub fn try_write(&mut self, data: T) -> SyncResult<SequenceNumber> {
        // Get next write position
        let write_pos = self.ring.write_pos.load(Ordering::Relaxed);
        let slot = self.ring.slot(write_pos);

        // Try to claim the slot for writing
        let current_seq = slot.sequence().load_acquire();
        if !slot.sequence().try_claim_for_write(current_seq) {
            return Err(SyncError::NoSlotsAvailable);
        }

        // Write data to slot (slot is now in WRITING state)
        unsafe {
            std::ptr::write(slot.data_ptr(), data);
        }

        // Memory barrier - ensure data write completes before sequence update
        std::sync::atomic::fence(Ordering::Release);

        // Get new sequence number and mark slot ready
        let new_sequence = self.ring.next_writer_sequence();
        slot.sequence().mark_ready(new_sequence)?;

        // Advance write position
        self.ring.write_pos.store(write_pos + 1, Ordering::Relaxed);

        Ok(new_sequence)
    }

    /// Force write to a specific slot (overwrites existing data)
    ///
    /// Use with caution - can cause readers to miss data
    pub fn force_write(&mut self, data: T, slot_index: usize) -> SyncResult<SequenceNumber> {
        if slot_index >= self.ring.capacity {
            return Err(SyncError::NoSlotsAvailable);
        }

        let slot = self.ring.slot(slot_index);

        // Force claim slot (override any existing state)
        let current_seq = slot.sequence().load_relaxed();
        slot.sequence().store_relaxed(special::WRITING);

        // Write data
        unsafe {
            // Drop existing data if it was ready
            if AtomicSequence::is_ready(current_seq) {
                std::ptr::drop_in_place(slot.data_ptr());
            }
            std::ptr::write(slot.data_ptr(), data);
        }

        // Memory barrier and mark ready
        std::sync::atomic::fence(Ordering::Release);
        let new_sequence = self.ring.next_writer_sequence();
        slot.sequence().store_release(new_sequence);

        Ok(new_sequence)
    }

    /// Get the current write position
    pub fn write_position(&self) -> usize {
        self.ring.write_pos.load(Ordering::Relaxed)
    }

    /// Get the ring capacity
    pub fn capacity(&self) -> usize {
        self.ring.capacity()
    }
}

/// Reader handle for SWMR ring buffer
///
/// Each reader maintains its own position and can read at its own pace.
/// Multiple readers can exist simultaneously without coordination.
pub struct ReaderHandle<'a, T> {
    ring: &'a SWMRRing<T>,
    read_pos: usize,
    last_sequence: SequenceNumber,
}

impl<'a, T: Clone> ReaderHandle<'a, T> {
    /// Try to read the next available data
    ///
    /// Returns None if no new data is available
    pub fn try_read(&mut self) -> SyncResult<Option<(T, SequenceNumber)>> {
        let slot = self.ring.slot(self.read_pos);
        let checker = SequenceChecker::new(Arc::new(slot.sequence().clone()));

        // Use sequence checker to ensure consistent read
        match checker.consistent_read(|| unsafe { (*slot.data_ptr()).clone() }, 3) {
            Ok(data) => {
                let sequence = slot.sequence().load_acquire();

                // Only advance if this is newer data
                if sequence > self.last_sequence {
                    self.last_sequence = sequence;
                    self.read_pos += 1;
                    Ok(Some((data, sequence)))
                } else {
                    Ok(None) // No new data
                }
            }
            Err(SyncError::NoSlotsAvailable) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Read data from a specific slot (for random access)
    pub fn read_slot(&self, slot_index: usize) -> SyncResult<Option<(T, SequenceNumber)>> {
        if slot_index >= self.ring.capacity {
            return Err(SyncError::NoSlotsAvailable);
        }

        let slot = self.ring.slot(slot_index);
        let checker = SequenceChecker::new(Arc::new(slot.sequence().clone()));

        match checker.consistent_read(|| unsafe { (*slot.data_ptr()).clone() }, 3) {
            Ok(data) => {
                let sequence = slot.sequence().load_acquire();
                if AtomicSequence::is_ready(sequence) {
                    Ok(Some((data, sequence)))
                } else {
                    Ok(None)
                }
            }
            Err(SyncError::NoSlotsAvailable) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Skip ahead to the latest available data
    pub fn skip_to_latest(&mut self) -> SyncResult<()> {
        let write_pos = self.ring.write_pos.load(Ordering::Acquire);

        // Find the most recent slot with ready data
        for i in 0..self.ring.capacity {
            let check_pos = write_pos.wrapping_sub(i + 1);
            let slot = self.ring.slot(check_pos);
            let sequence = slot.sequence().load_acquire();

            if AtomicSequence::is_ready(sequence) && sequence > self.last_sequence {
                self.read_pos = check_pos + 1;
                self.last_sequence = sequence;
                return Ok(());
            }
        }

        Ok(()) // No newer data found
    }

    /// Get the current read position
    pub fn read_position(&self) -> usize {
        self.read_pos
    }

    /// Get the last sequence number read
    pub fn last_sequence(&self) -> SequenceNumber {
        self.last_sequence
    }

    /// Get how far behind the writer this reader is
    pub fn lag(&self) -> usize {
        let write_pos = self.ring.write_pos.load(Ordering::Relaxed);
        write_pos.saturating_sub(self.read_pos)
    }

    /// Check if reader has fallen too far behind (data might be lost)
    pub fn is_too_slow(&self) -> bool {
        self.lag() >= self.ring.capacity
    }
}

/// Shared SWMR ring that can be used across threads/processes
pub struct SharedSWMRRing<T> {
    ring: Arc<SWMRRing<T>>,
}

impl<T> SharedSWMRRing<T> {
    /// Create a new shared SWMR ring
    pub fn new(capacity: usize) -> SyncResult<Self> {
        Ok(Self {
            ring: Arc::new(SWMRRing::new(capacity)?),
        })
    }

    /// Get a writer handle (only one should exist)
    pub fn writer(&self) -> WriterHandle<'_, T> {
        self.ring.writer()
    }

    /// Get a reader handle (multiple can exist)
    pub fn reader(&self) -> ReaderHandle<'_, T> {
        self.ring.reader()
    }

    /// Clone for sharing across threads
    pub fn clone(&self) -> Self {
        Self {
            ring: Arc::clone(&self.ring),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_swmr_basic() {
        let ring = SWMRRing::<i32>::new(4).unwrap();
        let mut writer = ring.writer();
        let mut reader = ring.reader();

        // Write some data
        let seq1 = writer.try_write(42).unwrap();
        let seq2 = writer.try_write(84).unwrap();

        // Read it back
        let (data1, read_seq1) = reader.try_read().unwrap().unwrap();
        assert_eq!(data1, 42);
        assert_eq!(read_seq1, seq1);

        let (data2, read_seq2) = reader.try_read().unwrap().unwrap();
        assert_eq!(data2, 84);
        assert_eq!(read_seq2, seq2);

        // No more data
        assert!(reader.try_read().unwrap().is_none());
    }

    #[test]
    fn test_swmr_multiple_readers() {
        let ring = Arc::new(SWMRRing::<u64>::new(8).unwrap());
        let barrier = Arc::new(Barrier::new(3)); // 1 writer + 2 readers

        let writer_ring = ring.clone();
        let writer_barrier = barrier.clone();
        let writer_handle = thread::spawn(move || {
            let mut writer = writer_ring.writer();
            writer_barrier.wait();

            for i in 0..10 {
                writer.try_write(i).unwrap();
                thread::sleep(Duration::from_millis(1));
            }
        });

        let reader_handles: Vec<_> = (0..2)
            .map(|reader_id| {
                let reader_ring = ring.clone();
                let reader_barrier = barrier.clone();
                thread::spawn(move || {
                    let mut reader = reader_ring.reader();
                    reader_barrier.wait();

                    let mut read_data = Vec::new();
                    for _ in 0..20 {
                        // Try to read more than writer produces
                        if let Some((data, _seq)) = reader.try_read().unwrap() {
                            read_data.push(data);
                        }
                        thread::sleep(Duration::from_millis(2));
                    }
                    (reader_id, read_data)
                })
            })
            .collect();

        writer_handle.join().unwrap();

        let results: Vec<_> = reader_handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();

        // Each reader should have read some data
        for (reader_id, data) in results {
            println!("Reader {} read {} items", reader_id, data.len());
            assert!(!data.is_empty());
        }
    }

    #[test]
    fn test_swmr_slot_access() {
        let ring = SWMRRing::<String>::new(4).unwrap();
        let mut writer = ring.writer();
        let reader = ring.reader();

        writer.force_write("slot0".to_string(), 0).unwrap();
        writer.force_write("slot2".to_string(), 2).unwrap();

        let (data0, _) = reader.read_slot(0).unwrap().unwrap();
        assert_eq!(data0, "slot0");

        let (data2, _) = reader.read_slot(2).unwrap().unwrap();
        assert_eq!(data2, "slot2");

        // Slot 1 should be empty
        assert!(reader.read_slot(1).unwrap().is_none());
    }

    #[test]
    fn test_reader_lag_tracking() {
        let ring = SWMRRing::<u32>::new(4).unwrap();
        let mut writer = ring.writer();
        let mut reader = ring.reader();

        // Fill the ring
        for i in 0..6 {
            writer.try_write(i).unwrap();
        }

        // Reader should be behind
        assert!(reader.lag() >= 4);
        assert!(reader.is_too_slow());

        // Skip to latest
        reader.skip_to_latest().unwrap();
        assert!(reader.lag() < 4);
        assert!(!reader.is_too_slow());
    }
}
