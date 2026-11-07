//! Topic statistics tracking

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

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
    /// Cumulative consumed latency in ns (for running average)
    total_latency_ns: AtomicU64,
    /// Peak single-message latency in ns
    peak_latency_ns: AtomicU64,
    /// Nanosecond timestamp of the first published message (UNIX epoch base)
    first_published_at_ns: AtomicU64,
}
impl TopicStats {
    /// Increment message published count and update sequence
    pub fn record_published(&self, message_size: usize) -> u64 {
        self.record_published_stats(message_size);
        self.current_sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Increment message published count and update average size, without
    /// touching `current_sequence`. Used by the ring implementations after
    /// the caller has already claimed a sequence number atomically via
    /// `current_sequence.fetch_add` at message-construction time, so the
    /// counter isn't advanced twice per message.
    pub fn record_published_stats(&self, message_size: usize) {
        self.messages_published.fetch_add(1, Ordering::Relaxed);

        // Record the first published timestamp once (for throughput calc).
        // Load-guarded so the clock read only happens on the first message.
        if self.first_published_at_ns.load(Ordering::Relaxed) == 0 {
            let _ = self.first_published_at_ns.compare_exchange(
                0,
                now_ns(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }

        // Update running average message size
        let current_avg = self.avg_message_size.load(Ordering::Relaxed);
        let published_count = self.messages_published.load(Ordering::Relaxed);
        let new_avg =
            ((current_avg * (published_count - 1)) + message_size as u64) / published_count;
        self.avg_message_size.store(new_avg, Ordering::Relaxed);
    }

    /// Increment message consumed count and record end-to-end latency
    /// (publish timestamp to consume time, in nanoseconds)
    pub fn record_consumed(&self, latency_ns: u64) {
        self.messages_consumed.fetch_add(1, Ordering::Relaxed);
        self.total_latency_ns
            .fetch_add(latency_ns, Ordering::Relaxed);
        self.peak_latency_ns
            .fetch_max(latency_ns, Ordering::Relaxed);
    }

    /// Get the average consumed latency in ns (0 if nothing consumed yet)
    pub fn average_latency_ns(&self) -> u64 {
        let consumed = self.messages_consumed.load(Ordering::Relaxed);
        if consumed == 0 {
            return 0;
        }
        self.total_latency_ns.load(Ordering::Relaxed) / consumed
    }

    /// Get the peak consumed latency in ns (0 if nothing consumed yet)
    pub fn peak_latency_ns(&self) -> u64 {
        self.peak_latency_ns.load(Ordering::Relaxed)
    }

    /// Cumulative average throughput in Mbps since the first published
    /// message (0 if nothing published yet, or elapsed time is zero)
    pub fn throughput_mbps(&self) -> f64 {
        let published = self.messages_published.load(Ordering::Relaxed);
        if published == 0 {
            return 0.0;
        }
        let first_at = self.first_published_at_ns.load(Ordering::Relaxed);
        if first_at == 0 {
            return 0.0;
        }
        let elapsed_secs = now_ns().saturating_sub(first_at) as f64 / 1_000_000_000.0;
        if elapsed_secs <= 0.0 {
            return 0.0;
        }
        let bytes = self.avg_message_size.load(Ordering::Relaxed) * published;
        (bytes as f64 * 8.0) / elapsed_secs / 1_000_000.0
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

/// Current wall-clock time in nanoseconds since the UNIX epoch
fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
