//! Zero-Copy Message Schema Strategy
//! 
//! Provides a unified interface for zero-copy message serialization:
//! 
//! **Zero-copy Schema (High Performance)**: For FlatBuffers/Cap'n Proto
//! - Direct field access without deserialization
//! - Schema-aware with compile-time validation  
//! - Optimal for high-frequency sensor data and real-time systems
//! - Memory-efficient with shared buffer pools
//!
//! The system is designed to be extensible and focuses exclusively on
//! zero-copy patterns for maximum performance.

use crate::error::Result;

// Re-export format modules
pub mod zero_copy;
pub mod registry;
pub mod traits;
pub mod official_flatbuffers;

// Export key traits and types for external use  
pub use traits::{ZeroCopyFormat, ZeroCopyAccessor as ZeroCopyAccess, SchemaValidator, ZeroCopyBuilder as BufferBuilder, SchemaCompatibility, FieldType};
pub use zero_copy::*;
pub use registry::{ZeroCopyFormatRegistry, UseCase, SchemaInfo as RegistrySchemaInfo};
pub use official_flatbuffers::OfficialFlatBufferFormat;

/// Schema format type identifier (zero-copy only)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FormatType {
    /// FlatBuffers (zero-copy)
    FlatBuffers = 1,
    /// Cap'n Proto (zero-copy)
    CapnProto = 2,
    /// Custom zero-copy format (user-defined)
    Custom = 255,
}

impl FormatType {
    /// All supported formats are zero-copy by definition
    pub fn is_zero_copy(self) -> bool {
        true
    }
    
    /// Zero-copy formats never require deserialization
    pub fn requires_deserialization(self) -> bool {
        false
    }
    
    /// Get format name for debugging/logging
    pub fn name(self) -> &'static str {
        match self {
            FormatType::FlatBuffers => "FlatBuffers",
            FormatType::CapnProto => "CapnProto",
            FormatType::Custom => "Custom",
        }
    }
}

// SchemaInfo is defined in registry.rs

/// Wrapper for formatted messages with schema information
#[derive(Debug)]
pub struct FormattedMessage {
    pub schema: registry::SchemaInfo,
    pub buffer: Vec<u8>,
}

impl FormattedMessage {
    pub fn new(schema: registry::SchemaInfo, buffer: Vec<u8>) -> Self {
        Self { schema, buffer }
    }
    
    /// Get the underlying buffer for zero-copy access
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }
    
    /// Check if the message format matches expected schema
    pub fn validate_schema(&self, expected: &registry::SchemaInfo) -> Result<()> {
        if self.schema.is_compatible(expected) {
            Ok(())
        } else {
            Err(crate::error::RenoirError::invalid_parameter(
                "schema",
                &format!(
                    "Schema mismatch: expected {} v{}, got {} v{}",
                    expected.schema_name, expected.schema_version,
                    self.schema.schema_name, self.schema.schema_version
                )
            ))
        }
    }
}