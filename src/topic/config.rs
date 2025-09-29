//! Topic configuration and QoS settings

use crate::message_formats::{FormatType, registry::SchemaInfo};

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

/// Message format configuration for a topic
#[derive(Debug, Clone)]
pub struct TopicMessageFormat {
    /// Message format type
    pub format_type: FormatType,
    /// Optional schema for validation and compatibility
    pub schema: Option<SchemaInfo>,
    /// Whether to enforce schema validation
    pub enforce_schema: bool,
    /// Allow format auto-detection from message headers
    pub auto_detect_format: bool,
}

impl Default for TopicMessageFormat {
    fn default() -> Self {
        Self {
            format_type: FormatType::FlatBuffers, // Default to zero-copy
            schema: None,
            enforce_schema: false, // Lenient by default
            auto_detect_format: true, // Allow mixed formats
        }
    }
}

impl TopicMessageFormat {
    /// Create format config for FlatBuffers (recommended)
    pub fn flatbuffers(schema_name: &str, schema_version: u32) -> Self {
        Self {
            format_type: FormatType::FlatBuffers,
            schema: Some(SchemaInfo::new(
                FormatType::FlatBuffers,
                schema_name.to_string(),
                schema_version,
                0, // Schema hash would be computed from actual schema
            )),
            enforce_schema: true,
            auto_detect_format: false,
        }
    }
    
    /// Create format config for zero-copy FlatBuffers (with existing schema)
    pub fn flatbuffers_with_schema(schema: SchemaInfo) -> Self {
        Self {
            format_type: FormatType::FlatBuffers,
            schema: Some(schema),
            enforce_schema: true,
            auto_detect_format: false,
        }
    }
    
    /// Create format config for Cap'n Proto
    pub fn capnproto(schema_name: &str, schema_version: u32) -> Self {
        Self {
            format_type: FormatType::CapnProto,
            schema: Some(SchemaInfo::new(
                FormatType::CapnProto,
                schema_name.to_string(),
                schema_version,
                0, // Schema hash would be computed from actual schema
            )),
            enforce_schema: true,
            auto_detect_format: false,
        }
    }
    
    /// Create format config for custom zero-copy format
    pub fn custom(schema: SchemaInfo) -> Self {
        Self {
            format_type: FormatType::Custom,
            schema: Some(schema),
            enforce_schema: true,
            auto_detect_format: false,
        }
    }
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
    /// Message format configuration
    pub message_format: TopicMessageFormat,
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
            message_format: TopicMessageFormat::default(),
        }
    }
}

impl TopicConfig {
    /// Create a new topic configuration with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
    
    /// Set message format for zero-copy FlatBuffers
    pub fn with_flatbuffers_format(mut self, schema_name: &str, schema_version: u32) -> Self {
        self.message_format = TopicMessageFormat::flatbuffers(schema_name, schema_version);
        self
    }
    
    /// Set message format for zero-copy FlatBuffers with existing schema
    pub fn with_flatbuffers_schema(mut self, schema: SchemaInfo) -> Self {
        self.message_format = TopicMessageFormat::flatbuffers_with_schema(schema);
        self
    }
    
    /// Set message format for Cap'n Proto
    pub fn with_capnproto_format(mut self, schema_name: &str, schema_version: u32) -> Self {
        self.message_format = TopicMessageFormat::capnproto(schema_name, schema_version);
        self
    }
    
    /// Set message format for custom zero-copy format
    pub fn with_custom_format(mut self, schema: SchemaInfo) -> Self {
        self.message_format = TopicMessageFormat::custom(schema);
        self
    }
    
    /// Set communication pattern
    pub fn with_pattern(mut self, pattern: TopicPattern) -> Self {
        self.pattern = pattern;
        self
    }
    
    /// Set ring buffer capacity (will be rounded up to next power of 2)
    pub fn with_ring_capacity(mut self, capacity: usize) -> Self {
        self.ring_capacity = capacity.next_power_of_two();
        self
    }
    
    /// Enable shared buffer pool for large messages
    pub fn with_shared_pool(mut self, threshold: usize) -> Self {
        self.use_shared_pool = true;
        self.shared_pool_threshold = threshold;
        self
    }
    
    /// Set QoS settings
    pub fn with_qos(mut self, qos: TopicQoS) -> Self {
        self.qos = qos;
        self
    }
    
    /// Check if configuration is valid
    pub fn validate(&self) -> Result<(), crate::error::RenoirError> {
        use crate::error::RenoirError;
        
        if self.name.is_empty() {
            return Err(RenoirError::invalid_parameter("name", "Topic name cannot be empty"));
        }
        
        if !self.ring_capacity.is_power_of_two() {
            return Err(RenoirError::invalid_parameter(
                "ring_capacity", 
                "Ring capacity must be power of 2"
            ));
        }
        
        if self.max_payload_size == 0 {
            return Err(RenoirError::invalid_parameter(
                "max_payload_size",
                "Max payload size must be greater than 0"
            ));
        }
        
        Ok(())
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