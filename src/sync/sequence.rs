//! Atomic sequence numbers with memory barriers for consistent slot access
//!
//! This module provides the core sequencing primitives for SWMR patterns.
//! Writers increment sequence numbers to indicate new data availability.
//! Readers check sequence consistency before/after accessing slot data.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::SyncResult;

/// Sequence number type for strong typing
pub type SequenceNumber = u64;

/// Special sequence number values
pub mod special {
    use super::SequenceNumber;

    /// Indicates slot is empty/unused
    pub const EMPTY: SequenceNumber = 0;
    /// Indicates slot is being written (writer has claimed but not finished)
    pub const WRITING: SequenceNumber = 1;
    /// First valid sequence number for data
    pub const FIRST_DATA: SequenceNumber = 2;
}

/// Atomic sequence number with robotics-optimized memory ordering
#[derive(Debug)]
pub struct AtomicSequence {
    /// The atomic sequence counter
    value: AtomicU64,
}

impl AtomicSequence {
    /// Create a new atomic sequence starting at EMPTY
    pub fn new() -> Self {
        Self {
            value: AtomicU64::new(special::EMPTY),
        }
    }

    /// Create a new atomic sequence with initial value
    pub fn with_initial(initial: SequenceNumber) -> Self {
        Self {
            value: AtomicU64::new(initial),
        }
    }

    /// Load the current sequence number with acquire ordering
    ///
    /// Use this for readers to get a consistent view before reading data
    pub fn load_acquire(&self) -> SequenceNumber {
        self.value.load(Ordering::Acquire)
    }

    /// Load the current sequence number with relaxed ordering
    ///
    /// Use this for quick checks where consistency isn't critical
    pub fn load_relaxed(&self) -> SequenceNumber {
        self.value.load(Ordering::Relaxed)
    }

    /// Store a new sequence number with release ordering
    ///
    /// Use this by writers after data has been written to slot
    pub fn store_release(&self, seq: SequenceNumber) {
        self.value.store(seq, Ordering::Release);
    }

    /// Store a new sequence number with relaxed ordering
    pub fn store_relaxed(&self, seq: SequenceNumber) {
        self.value.store(seq, Ordering::Relaxed);
    }

    /// Compare and swap with acquire-release ordering
    ///
    /// Returns the previous value and whether the swap succeeded
    pub fn compare_exchange_weak(
        &self,
        current: SequenceNumber,
        new: SequenceNumber,
    ) -> Result<SequenceNumber, SequenceNumber> {
        self.value
            .compare_exchange_weak(current, new, Ordering::AcqRel, Ordering::Acquire)
    }

    /// Attempt to claim slot for writing
    ///
    /// Tries to change from EMPTY -> WRITING, or from old_seq -> WRITING
    pub fn try_claim_for_write(&self, expected_seq: SequenceNumber) -> bool {
        self.compare_exchange_weak(expected_seq, special::WRITING)
            .is_ok()
    }

    /// Mark slot as ready with new sequence number
    ///
    /// Changes from WRITING -> new_seq with release ordering
    pub fn mark_ready(&self, new_seq: SequenceNumber) -> SyncResult<()> {
        match self.compare_exchange_weak(special::WRITING, new_seq) {
            Ok(_) => Ok(()),
            Err(_) => {
                // Slot wasn't in WRITING state - this is a bug
                panic!("mark_ready called on slot not in WRITING state");
            }
        }
    }

    /// Fetch and increment sequence number
    ///
    /// Atomically increments and returns the previous value
    pub fn fetch_increment(&self) -> SequenceNumber {
        self.value.fetch_add(1, Ordering::AcqRel)
    }

    /// Check if sequence indicates data is ready (not EMPTY or WRITING)
    pub fn is_ready(seq: SequenceNumber) -> bool {
        seq >= special::FIRST_DATA
    }

    /// Check if sequence indicates slot is empty
    pub fn is_empty(seq: SequenceNumber) -> bool {
        seq == special::EMPTY
    }

    /// Check if sequence indicates slot is being written
    pub fn is_writing(seq: SequenceNumber) -> bool {
        seq == special::WRITING
    }
}

impl Default for AtomicSequence {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for AtomicSequence {
    fn clone(&self) -> Self {
        Self::with_initial(self.load_relaxed())
    }
}

/// Sequence consistency checker for readers
///
/// Implements the classic "read before/after sequence check" pattern
/// to ensure data consistency in lock-free scenarios
#[derive(Debug)]
pub struct SequenceChecker {
    sequence: Arc<AtomicSequence>,
}

impl SequenceChecker {
    /// Create a new checker for the given sequence
    pub fn new(sequence: Arc<AtomicSequence>) -> Self {
        Self { sequence }
    }

    /// Begin a read operation, returning the sequence to check after
    pub fn begin_read(&self) -> SequenceNumber {
        self.sequence.load_acquire()
    }

    /// End a read operation, checking sequence consistency
    ///
    /// Returns Ok(()) if the data read is consistent, Err if retry needed
    pub fn end_read(&self, start_seq: SequenceNumber) -> SyncResult<SequenceNumber> {
        let end_seq = self.sequence.load_acquire();

        if start_seq != end_seq {
            Err(super::SyncError::SequenceInconsistent {
                expected: start_seq,
                actual: end_seq,
            })
        } else if AtomicSequence::is_writing(start_seq) {
            // Started read while writer was active
            Err(super::SyncError::CorruptedBuffer)
        } else {
            Ok(end_seq)
        }
    }

    /// Convenience method to perform a read with automatic retry
    ///
    /// Calls read_fn repeatedly until sequence is consistent or max_retries reached
    pub fn consistent_read<T, F>(&self, mut read_fn: F, max_retries: usize) -> SyncResult<T>
    where
        F: FnMut() -> T,
    {
        for _ in 0..max_retries {
            let start_seq = self.begin_read();

            // Don't try to read if slot is empty or being written
            if !AtomicSequence::is_ready(start_seq) {
                return Err(super::SyncError::NoSlotsAvailable);
            }

            let data = read_fn();

            match self.end_read(start_seq) {
                Ok(_) => return Ok(data),
                Err(super::SyncError::SequenceInconsistent { .. }) => continue,
                Err(e) => return Err(e),
            }
        }

        Err(super::SyncError::SequenceInconsistent {
            expected: 0,
            actual: self.sequence.load_acquire(),
        })
    }
}

/// Global sequence counter for coordinating multiple SWMR rings
#[derive(Debug)]
pub struct GlobalSequenceGenerator {
    counter: AtomicU64,
}

impl GlobalSequenceGenerator {
    /// Create a new global sequence generator
    pub fn new() -> Self {
        Self {
            counter: AtomicU64::new(special::FIRST_DATA),
        }
    }

    /// Get the next sequence number
    pub fn next(&self) -> SequenceNumber {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Get the current sequence number without incrementing
    pub fn current(&self) -> SequenceNumber {
        self.counter.load(Ordering::Relaxed)
    }
}

impl Default for GlobalSequenceGenerator {
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    /// Thread-local sequence generator for per-thread efficiency
    static LOCAL_SEQUENCE: std::cell::RefCell<u64> = std::cell::RefCell::new(special::FIRST_DATA);
}

/// Get next sequence number from thread-local generator (fastest option)
///
/// Uses thread-local storage for maximum performance without atomic contention.
pub fn next_local_sequence() -> SequenceNumber {
    LOCAL_SEQUENCE.with(|seq| {
        let mut val = seq.borrow_mut();
        *val += 1;
        *val
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_atomic_sequence_basic() {
        let seq = AtomicSequence::new();
        assert_eq!(seq.load_relaxed(), special::EMPTY);

        seq.store_release(42);
        assert_eq!(seq.load_acquire(), 42);
    }

    #[test]
    fn test_sequence_states() {
        assert!(AtomicSequence::is_empty(special::EMPTY));
        assert!(AtomicSequence::is_writing(special::WRITING));
        assert!(AtomicSequence::is_ready(special::FIRST_DATA));
        assert!(AtomicSequence::is_ready(100));
    }

    #[test]
    fn test_claim_and_ready() {
        let seq = AtomicSequence::new();

        // Claim empty slot
        assert!(seq.try_claim_for_write(special::EMPTY));
        assert_eq!(seq.load_relaxed(), special::WRITING);

        // Mark ready
        seq.mark_ready(special::FIRST_DATA).unwrap();
        assert_eq!(seq.load_relaxed(), special::FIRST_DATA);
    }

    #[test]
    fn test_consistency_checker() {
        let seq = Arc::new(AtomicSequence::with_initial(special::FIRST_DATA));
        let checker = SequenceChecker::new(seq.clone());

        let result = checker.consistent_read(|| 42, 5);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_global_sequence_generator() {
        let gen = GlobalSequenceGenerator::new();
        let first = gen.next();
        let second = gen.next();
        assert_eq!(second, first + 1);
    }

    #[test]
    fn test_concurrent_access() {
        let seq = Arc::new(AtomicSequence::new());
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let seq = seq.clone();
                thread::spawn(move || {
                    for _i in 0..100 {
                        let _ = seq.fetch_increment();
                        thread::yield_now();
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Should have incremented 4 * 100 = 400 times
        assert_eq!(seq.load_relaxed(), 400);
    }
}
