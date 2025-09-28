//! Topic statistics tracking

use std::sync::atomic::{AtomicU64, AtomicU32, Ordering};

/// Topic statistics
#[derive(Debug, Default)]
pub struct TopicStats {
    /// Total messages published
    pub messages_published: AtomicU64,
    /// Total messages consumed
    pub messages_consumed: AtomicU64,
    /// Messages dropped due to full buffer
    pub messages_dropped: AtomicU64,
    /// Current sequence number
    pub current_sequence: AtomicU64,
    /// Number of active publishers
    pub active_publishers: AtomicU32,
    /// Number of active subscribers
    pub active_subscribers: AtomicU32,
    /// Average message size in bytes
    pub avg_message_size: AtomicU64,
    /// Peak message rate (messages/second)
    pub peak_message_rate: AtomicU64,
}

impl TopicStats {
    /// Increment message published count and update sequence
    pub fn record_published(&self, message_size: usize) -> u64 {
        self.messages_published.fetch_add(1, Ordering::Relaxed);
        
        // Update running average message size
        let current_avg = self.avg_message_size.load(Ordering::Relaxed);
        let published_count = self.messages_published.load(Ordering::Relaxed);
        let new_avg = ((current_avg * (published_count - 1)) + message_size as u64) / published_count;
        self.avg_message_size.store(new_avg, Ordering::Relaxed);
        
        self.current_sequence.fetch_add(1, Ordering::SeqCst) + 1
    }
    
    /// Increment message consumed count
    pub fn record_consumed(&self) {
        self.messages_consumed.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Increment dropped message count
    pub fn record_dropped(&self) {
        self.messages_dropped.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Add a publisher
    pub fn add_publisher(&self) -> u32 {
        self.active_publishers.fetch_add(1, Ordering::Relaxed) + 1
    }
    
    /// Remove a publisher
    pub fn remove_publisher(&self) -> u32 {
        self.active_publishers.fetch_sub(1, Ordering::Relaxed) - 1
    }
    
    /// Add a subscriber
    pub fn add_subscriber(&self) -> u32 {
        self.active_subscribers.fetch_add(1, Ordering::Relaxed) + 1
    }
    
    /// Remove a subscriber
    pub fn remove_subscriber(&self) -> u32 {
        self.active_subscribers.fetch_sub(1, Ordering::Relaxed) - 1
    }
}