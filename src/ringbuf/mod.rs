//! High-performance lock-free ring buffer implementations

pub mod basic;
pub mod sequenced;

#[cfg(test)]
mod tests;

// Re-export main types for convenience
pub use basic::{RingBuffer, Producer, Consumer};
pub use sequenced::{SequencedRingBuffer, ClaimGuard};