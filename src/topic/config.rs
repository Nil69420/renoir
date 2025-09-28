//! Topic configuration and QoS settings

/// Topic communication pattern
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicPattern {
    /// Single producer, single consumer (most common for sensors)
    SPSC,
    /// Single producer, multiple consumers (broadcast)
    SPMC,
    /// Multiple producers, single consumer (aggregation)  
    MPSC,
    /// Multiple producers, multiple consumers (rare, but needed)
    MPMC,
}

/// Topic configuration
#[derive(Debug, Clone)]
pub struct TopicConfig {
    /// Topic name (must be unique)
    pub name: String,
    /// Communication pattern
    pub pattern: TopicPattern,
    /// Ring buffer capacity (must be power of 2)
    pub ring_capacity: usize,
    /// Maximum message payload size
    pub max_payload_size: usize,
    /// Whether to use shared buffer pool for large messages
    pub use_shared_pool: bool,
    /// Threshold for using shared pool (bytes)
    pub shared_pool_threshold: usize,
    /// Enable eventfd notifications
    pub enable_notifications: bool,
    /// Quality of Service settings
    pub qos: TopicQoS,
}

impl Default for TopicConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            pattern: TopicPattern::SPSC,
            ring_capacity: 1024,
            max_payload_size: 64 * 1024, // 64KB
            use_shared_pool: false,
            shared_pool_threshold: 4096, // 4KB
            enable_notifications: true,
            qos: TopicQoS::default(),
        }
    }
}

/// Quality of Service settings for topics
#[derive(Debug, Clone)]
pub struct TopicQoS {
    /// Message durability (keep last N messages)
    pub durability: u32,
    /// Message reliability (best effort vs reliable)
    pub reliability: Reliability,
    /// History depth for late-joining subscribers
    pub history_depth: u32,
}

impl Default for TopicQoS {
    fn default() -> Self {
        Self {
            durability: 1,
            reliability: Reliability::BestEffort,
            history_depth: 10,
        }
    }
}

/// Message reliability levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reliability {
    /// Best effort delivery (may drop messages)
    BestEffort,
    /// Reliable delivery (waits for acknowledgment)
    Reliable,
}