//! Statistics for topic manager

use std::sync::atomic::{AtomicUsize, Ordering};

/// Global topic manager statistics
#[derive(Debug, Default)]
pub struct TopicManagerStats {
    /// Total topics created
    pub topics_created: AtomicUsize,
    /// Total topics removed
    pub topics_removed: AtomicUsize,
    /// Total messages published across all topics
    pub total_messages_published: AtomicUsize,
    /// Total messages consumed across all topics
    pub total_messages_consumed: AtomicUsize,
    /// Peak message rate (messages/second)
    pub peak_message_rate: AtomicUsize,
}

impl TopicManagerStats {
    /// Create new statistics
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the number of active topics (created - removed)
    pub fn active_topics(&self) -> usize {
        let created = self.topics_created.load(Ordering::Relaxed);
        let removed = self.topics_removed.load(Ordering::Relaxed);
        created.saturating_sub(removed)
    }

    /// Get total messages published
    pub fn total_published(&self) -> usize {
        self.total_messages_published.load(Ordering::Relaxed)
    }

    /// Get total messages consumed
    pub fn total_consumed(&self) -> usize {
        self.total_messages_consumed.load(Ordering::Relaxed)
    }

    /// Get message throughput efficiency (consumed / published)
    pub fn throughput_efficiency(&self) -> f64 {
        let published = self.total_published();
        if published == 0 {
            return 1.0;
        }
        
        let consumed = self.total_consumed();
        consumed as f64 / published as f64
    }

    /// Get peak message rate
    pub fn peak_rate(&self) -> usize {
        self.peak_message_rate.load(Ordering::Relaxed)
    }

    /// Update peak message rate if necessary
    pub fn update_peak_rate(&self, current_rate: usize) {
        let mut peak = self.peak_message_rate.load(Ordering::Relaxed);
        
        while current_rate > peak {
            match self.peak_message_rate.compare_exchange_weak(
                peak,
                current_rate,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => peak = x,
            }
        }
    }

    /// Record a message publication
    pub fn record_publish(&self) {
        self.total_messages_published.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a message consumption
    pub fn record_consume(&self) {
        self.total_messages_consumed.fetch_add(1, Ordering::Relaxed);
    }

    /// Reset all statistics
    pub fn reset(&self) {
        self.topics_created.store(0, Ordering::Relaxed);
        self.topics_removed.store(0, Ordering::Relaxed);
        self.total_messages_published.store(0, Ordering::Relaxed);
        self.total_messages_consumed.store(0, Ordering::Relaxed);
        self.peak_message_rate.store(0, Ordering::Relaxed);
    }
}