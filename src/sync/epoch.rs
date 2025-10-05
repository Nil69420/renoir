//! Epoch-based memory reclamation for lock-free data structures
//!
//! This module implements a grace period / epoch-based reclamation system that
//! allows safe memory reclamation without expensive atomic operations on the
//! read path. It tracks per-reader progress to determine when old buffers
//! can be safely reclaimed.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

use super::sequence::SequenceNumber;

/// Epoch number type for strong typing
pub type EpochNumber = u64;

/// Special epoch values
pub mod epoch {
    use super::EpochNumber;

    /// Initial epoch number
    pub const INITIAL: EpochNumber = 0;
    /// Indicates reader is inactive
    pub const INACTIVE: EpochNumber = u64::MAX;
}

/// Per-reader tracking information stored in shared memory
#[repr(C)]
#[derive(Debug)]
struct ReaderEntry {
    /// Last sequence number seen by this reader
    last_sequence: AtomicU64,
    /// Current epoch this reader is operating in
    current_epoch: AtomicU64,
    /// Whether this reader is active
    active: AtomicU64,
}

impl ReaderEntry {
    fn new() -> Self {
        Self {
            last_sequence: AtomicU64::new(0),
            current_epoch: AtomicU64::new(epoch::INITIAL),
            active: AtomicU64::new(0),
        }
    }

    /// Mark reader as active in given epoch
    fn enter_epoch(&self, epoch: EpochNumber) {
        self.current_epoch.store(epoch, Ordering::Relaxed);
        self.active.store(1, Ordering::Release);
    }

    /// Mark reader as inactive
    fn leave_epoch(&self) {
        self.active.store(0, Ordering::Release);
        self.current_epoch.store(epoch::INACTIVE, Ordering::Relaxed);
    }

    /// Update the last sequence number seen
    fn update_sequence(&self, seq: SequenceNumber) {
        self.last_sequence.store(seq, Ordering::Relaxed);
    }

    /// Get current status
    fn status(&self) -> (bool, EpochNumber, SequenceNumber) {
        let active = self.active.load(Ordering::Acquire) != 0;
        let epoch = self.current_epoch.load(Ordering::Relaxed);
        let sequence = self.last_sequence.load(Ordering::Relaxed);
        (active, epoch, sequence)
    }
}

/// Manager for epoch-based reclamation
pub struct EpochManager {
    /// Current global epoch
    global_epoch: AtomicU64,
    /// Reader tracking table
    readers: Mutex<HashMap<usize, Arc<ReaderEntry>>>,
    /// Next reader ID to assign
    next_reader_id: AtomicUsize,
    /// Pending reclamations organized by epoch
    pending_reclaims: Mutex<HashMap<EpochNumber, Vec<Box<dyn EpochReclaim>>>>,
}

impl EpochManager {
    /// Create a new epoch manager
    pub fn new() -> Self {
        Self {
            global_epoch: AtomicU64::new(epoch::INITIAL),
            readers: Mutex::new(HashMap::new()),
            next_reader_id: AtomicUsize::new(1),
            pending_reclaims: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new reader, returning a tracker
    pub fn register_reader(&self) -> ReaderTracker<'_> {
        let reader_id = self.next_reader_id.fetch_add(1, Ordering::Relaxed);
        let entry = Arc::new(ReaderEntry::new());

        {
            let mut readers = self.readers.lock().unwrap();
            readers.insert(reader_id, entry.clone());
        }

        ReaderTracker {
            manager: self,
            reader_id,
            entry,
        }
    }

    /// Unregister a reader
    pub fn unregister_reader(&self, reader_id: usize) {
        let mut readers = self.readers.lock().unwrap();
        readers.remove(&reader_id);
    }

    /// Advance the global epoch
    pub fn advance_epoch(&self) -> EpochNumber {
        self.global_epoch.fetch_add(1, Ordering::AcqRel)
    }

    /// Get the current global epoch
    pub fn current_epoch(&self) -> EpochNumber {
        self.global_epoch.load(Ordering::Acquire)
    }

    /// Schedule an object for reclamation in the next safe epoch
    pub fn defer_reclaim<T: EpochReclaim + 'static>(&self, object: T) {
        let current_epoch = self.current_epoch();
        let mut pending = self.pending_reclaims.lock().unwrap();
        pending
            .entry(current_epoch)
            .or_insert_with(Vec::new)
            .push(Box::new(object));
    }

    /// Try to reclaim objects that are safe to reclaim
    ///
    /// This checks all readers and reclaims objects from epochs that
    /// no active reader is still using
    pub fn try_reclaim(&self) -> usize {
        let current_epoch = self.current_epoch();

        // Compute safe epoch first, before locking pending_reclaims
        let safe_epoch = self.compute_safe_epoch(current_epoch);

        let mut pending = self.pending_reclaims.lock().unwrap();
        let mut reclaimed_count = 0;

        // Collect epochs that are safe to reclaim
        let mut safe_epochs = Vec::new();
        for &epoch in pending.keys() {
            if epoch < safe_epoch {
                safe_epochs.push(epoch);
            }
        }

        // Reclaim objects from safe epochs
        for epoch in safe_epochs {
            if let Some(objects) = pending.remove(&epoch) {
                reclaimed_count += objects.len();
                // Explicitly call reclaim on each object
                for mut obj in objects {
                    obj.reclaim();
                }
            }
        }

        reclaimed_count
    }

    /// Compute the safe epoch for reclamation
    ///
    /// This is the minimum epoch among all active readers
    fn compute_safe_epoch(&self, current_epoch: EpochNumber) -> EpochNumber {
        let readers = self.readers.lock().unwrap();

        let mut min_epoch = None;

        for entry in readers.values() {
            let (active, reader_epoch, _sequence) = entry.status();
            if active && reader_epoch != epoch::INACTIVE {
                match min_epoch {
                    None => min_epoch = Some(reader_epoch),
                    Some(current_min) => min_epoch = Some(current_min.min(reader_epoch)),
                }
            }
        }

        match min_epoch {
            // If no active readers, all epochs up to and including current are safe
            None => current_epoch + 1,
            // If active readers, safe epoch is the minimum active epoch
            Some(min) => min,
        }
    }

    /// Get statistics about the reclamation system
    pub fn stats(&self) -> EpochStats {
        let current_epoch = self.current_epoch();

        // Get reader stats first
        let (active_readers, total_readers) = {
            let readers = self.readers.lock().unwrap();
            let active = readers.values().filter(|entry| entry.status().0).count();
            (active, readers.len())
        };

        // Get pending stats
        let pending_objects = {
            let pending = self.pending_reclaims.lock().unwrap();
            pending.values().map(|v| v.len()).sum()
        };

        // Compute safe epoch last (uses readers lock again)
        let safe_epoch = self.compute_safe_epoch(current_epoch);

        EpochStats {
            current_epoch,
            active_readers,
            total_readers,
            pending_reclamations: pending_objects,
            safe_epoch,
        }
    }

    /// Force reclamation of all objects (unsafe, for shutdown)
    pub unsafe fn force_reclaim_all(&self) -> usize {
        let mut pending = self.pending_reclaims.lock().unwrap();
        let total: usize = pending.values().map(|v| v.len()).sum();
        pending.clear();
        total
    }
}

impl Default for EpochManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about the epoch reclamation system
#[derive(Debug, Clone)]
pub struct EpochStats {
    /// Current global epoch
    pub current_epoch: EpochNumber,
    /// Number of currently active readers
    pub active_readers: usize,
    /// Total number of registered readers
    pub total_readers: usize,
    /// Number of objects pending reclamation
    pub pending_reclamations: usize,
    /// Earliest epoch that is safe to reclaim
    pub safe_epoch: EpochNumber,
}

/// Trait for objects that can be reclaimed by the epoch system
pub trait EpochReclaim: Send + Sync {
    /// Called when the object can be safely reclaimed
    fn reclaim(&mut self);
}

impl<T: Send + Sync> EpochReclaim for Box<T> {
    fn reclaim(&mut self) {
        // Box automatically drops its contents
    }
}

impl<T: Send + Sync> EpochReclaim for Vec<T> {
    fn reclaim(&mut self) {
        self.clear();
    }
}

/// A guard that implements EpochReclaim for raw pointers
pub struct PtrGuard<T> {
    ptr: NonNull<T>,
    _phantom: PhantomData<T>,
}

impl<T> PtrGuard<T> {
    /// Create a new pointer guard (takes ownership of the pointer)
    pub unsafe fn new(ptr: NonNull<T>) -> Self {
        Self {
            ptr,
            _phantom: PhantomData,
        }
    }

    /// Get the wrapped pointer
    pub fn as_ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }
}

impl<T: Send + Sync> EpochReclaim for PtrGuard<T> {
    fn reclaim(&mut self) {
        unsafe {
            std::ptr::drop_in_place(self.ptr.as_ptr());
            std::alloc::dealloc(self.ptr.as_ptr() as *mut u8, std::alloc::Layout::new::<T>());
        }
    }
}

unsafe impl<T: Send> Send for PtrGuard<T> {}
unsafe impl<T: Sync> Sync for PtrGuard<T> {}

/// Reader tracker for participating in epoch-based reclamation
pub struct ReaderTracker<'a> {
    manager: &'a EpochManager,
    reader_id: usize,
    entry: Arc<ReaderEntry>,
}

impl<'a> ReaderTracker<'a> {
    /// Enter a new epoch for reading
    ///
    /// Call this before starting read operations
    pub fn enter(&self) -> EpochNumber {
        let epoch = self.manager.current_epoch();
        self.entry.enter_epoch(epoch);
        epoch
    }

    /// Leave the current epoch
    ///
    /// Call this after finishing read operations
    pub fn leave(&self) {
        self.entry.leave_epoch();
    }

    /// Update the last sequence number seen by this reader
    pub fn update_sequence(&self, sequence: SequenceNumber) {
        self.entry.update_sequence(sequence);
    }

    /// Get reader statistics
    pub fn status(&self) -> ReaderStatus {
        let (active, epoch, sequence) = self.entry.status();
        ReaderStatus {
            reader_id: self.reader_id,
            active,
            current_epoch: epoch,
            last_sequence: sequence,
        }
    }

    /// Execute a closure within an epoch
    pub fn with_epoch<F, R>(&self, f: F) -> R
    where
        F: FnOnce(EpochNumber) -> R,
    {
        let epoch = self.enter();
        let result = f(epoch);
        self.leave();
        result
    }
}

impl Drop for ReaderTracker<'_> {
    fn drop(&mut self) {
        self.manager.unregister_reader(self.reader_id);
    }
}

/// Status information for a reader
#[derive(Debug, Clone)]
pub struct ReaderStatus {
    /// Reader ID
    pub reader_id: usize,
    /// Whether reader is currently active
    pub active: bool,
    /// Current epoch the reader is in
    pub current_epoch: EpochNumber,
    /// Last sequence number seen
    pub last_sequence: SequenceNumber,
}

/// Shared epoch manager that can be used across threads
pub struct SharedEpochManager {
    manager: Arc<EpochManager>,
}

impl SharedEpochManager {
    /// Create a new shared epoch manager
    pub fn new() -> Self {
        Self {
            manager: Arc::new(EpochManager::new()),
        }
    }

    /// Register a reader
    pub fn register_reader(&self) -> ReaderTracker<'_> {
        self.manager.register_reader()
    }

    /// Advance the global epoch
    pub fn advance_epoch(&self) -> EpochNumber {
        self.manager.advance_epoch()
    }

    /// Get current epoch
    pub fn current_epoch(&self) -> EpochNumber {
        self.manager.current_epoch()
    }

    /// Defer object reclamation
    pub fn defer_reclaim<T: EpochReclaim + 'static>(&self, object: T) {
        self.manager.defer_reclaim(object);
    }

    /// Try to reclaim safe objects
    pub fn try_reclaim(&self) -> usize {
        self.manager.try_reclaim()
    }

    /// Get statistics
    pub fn stats(&self) -> EpochStats {
        self.manager.stats()
    }

    /// Clone for sharing across threads
    pub fn clone(&self) -> Self {
        Self {
            manager: Arc::clone(&self.manager),
        }
    }
}

impl Default for SharedEpochManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    // Test object for reclamation
    struct TestObject {
        #[allow(dead_code)]
        data: Vec<u8>,
        reclaimed: Arc<AtomicUsize>,
    }

    impl EpochReclaim for TestObject {
        fn reclaim(&mut self) {
            self.reclaimed.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn test_epoch_basic() {
        let manager = EpochManager::new();

        assert_eq!(manager.current_epoch(), epoch::INITIAL);

        let new_epoch = manager.advance_epoch();
        assert_eq!(new_epoch, epoch::INITIAL);
        assert_eq!(manager.current_epoch(), epoch::INITIAL + 1);
    }

    #[test]
    fn test_reader_tracking() {
        let manager = EpochManager::new();
        let reader = manager.register_reader();

        let epoch = reader.enter();
        assert_eq!(epoch, epoch::INITIAL);

        let status = reader.status();
        assert!(status.active);
        assert_eq!(status.current_epoch, epoch::INITIAL);

        reader.leave();
        let status = reader.status();
        assert!(!status.active);
    }

    #[test]
    fn test_reclamation_basic() {
        let manager = EpochManager::new();
        let reclaimed_count = Arc::new(AtomicUsize::new(0));

        // Defer some objects for reclamation
        for i in 0..5 {
            let obj = TestObject {
                data: vec![i; 100],
                reclaimed: reclaimed_count.clone(),
            };
            manager.defer_reclaim(obj);
        }

        // Advance epoch and try to reclaim
        manager.advance_epoch();
        manager.advance_epoch(); // Need to advance past objects' epoch

        let reclaimed = manager.try_reclaim();
        assert_eq!(reclaimed, 5);
        assert_eq!(reclaimed_count.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn test_reader_prevents_reclamation() {
        let manager = EpochManager::new();
        let reader = manager.register_reader();
        let reclaimed_count = Arc::new(AtomicUsize::new(0));

        // Reader enters current epoch
        reader.enter();

        // Defer object for reclamation
        let obj = TestObject {
            data: vec![42; 100],
            reclaimed: reclaimed_count.clone(),
        };
        manager.defer_reclaim(obj);

        // Advance epoch but reader is still in old epoch
        manager.advance_epoch();

        // Should not be able to reclaim yet
        let reclaimed = manager.try_reclaim();
        assert_eq!(reclaimed, 0);
        assert_eq!(reclaimed_count.load(Ordering::Relaxed), 0);

        // Reader leaves epoch
        reader.leave();

        // Now should be able to reclaim
        let reclaimed = manager.try_reclaim();
        assert_eq!(reclaimed, 1);
        assert_eq!(reclaimed_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_multiple_readers() {
        let manager = Arc::new(EpochManager::new());
        let barrier = Arc::new(Barrier::new(3)); // 2 readers + main thread
        let reclaimed_count = Arc::new(AtomicUsize::new(0));

        // Start two readers
        let handles: Vec<_> = (0..2)
            .map(|_i| {
                let manager = manager.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let reader = manager.register_reader();
                    reader.enter(); // Enter before sync
                    barrier.wait(); // Sync start

                    thread::sleep(Duration::from_millis(100)); // Hold epoch for longer
                    reader.leave();
                })
            })
            .collect();

        barrier.wait(); // Wait for readers to start and enter

        // Defer object while readers are active
        let obj = TestObject {
            data: vec![99; 100],
            reclaimed: reclaimed_count.clone(),
        };
        manager.defer_reclaim(obj);

        // Advance epoch
        manager.advance_epoch();

        // Should not reclaim while readers are active
        // Give a small delay to ensure readers are still in their epochs
        thread::sleep(Duration::from_millis(10));
        assert_eq!(manager.try_reclaim(), 0);

        // Wait for readers to finish
        for handle in handles {
            handle.join().unwrap();
        }

        // Now should be able to reclaim
        assert_eq!(manager.try_reclaim(), 1);
        assert_eq!(reclaimed_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_epoch_statistics() {
        let manager = EpochManager::new();
        let _reader1 = manager.register_reader();
        let reader2 = manager.register_reader();

        reader2.enter();

        // Defer some objects
        for i in 0..3 {
            let obj = TestObject {
                data: vec![i; 10],
                reclaimed: Arc::new(AtomicUsize::new(0)),
            };
            manager.defer_reclaim(obj);
        }

        let stats = manager.stats();
        assert_eq!(stats.total_readers, 2);
        assert_eq!(stats.active_readers, 1);
        assert_eq!(stats.pending_reclamations, 3);
    }
}
