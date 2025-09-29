//! Synchronization primitives for high-performance robotics applications
//!
//! This module provides lock-free synchronization patterns optimized for robotics use cases,
//! particularly Single Writer Multiple Reader (SWMR) scenarios common in sensor processing.
//!
//! Key features:
//! - Atomic sequence numbers with memory barriers for consistency
//! - SWMR ring buffers for sensor → multiple consumers
//! - Epoch-based memory reclamation without expensive atomic ops on read path
//! - eventfd-based notifications for efficient wakeups
//! - No locks on hot path for maximum real-time performance

pub mod sequence;
pub mod swmr;
pub mod epoch;
pub mod notify;

pub use sequence::{AtomicSequence, SequenceNumber};
pub use swmr::{SWMRRing, SWMRSlot, ReaderHandle, WriterHandle};
pub use epoch::{EpochManager, EpochReclaim, ReaderTracker};
pub use notify::{EventNotifier, NotificationGroup, NotifyCondition};

/// Common synchronization error types
#[derive(Debug, Clone, PartialEq)]
pub enum SyncError {
    /// Sequence number inconsistency detected
    SequenceInconsistent {
        expected: u64,
        actual: u64,
    },
    /// Reader fell too far behind
    ReaderTooSlow {
        reader_seq: u64,
        writer_seq: u64,
    },
    /// No slots available for writing
    NoSlotsAvailable,
    /// Buffer was corrupted or partially written
    CorruptedBuffer,
    /// Notification system failure
    NotificationFailed,
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::SequenceInconsistent { expected, actual } => {
                write!(f, "Sequence inconsistent: expected {}, got {}", expected, actual)
            }
            SyncError::ReaderTooSlow { reader_seq, writer_seq } => {
                write!(f, "Reader too slow: reader at {}, writer at {}", reader_seq, writer_seq)
            }
            SyncError::NoSlotsAvailable => write!(f, "No slots available for writing"),
            SyncError::CorruptedBuffer => write!(f, "Buffer corrupted or partially written"),
            SyncError::NotificationFailed => write!(f, "Notification system failed"),
        }
    }
}

impl std::error::Error for SyncError {}

/// Result type for synchronization operations
pub type SyncResult<T> = Result<T, SyncError>;