//! Core traits for zero-copy message formats
//!
//! Provides extensible framework for zero-copy serialization strategies

use super::registry::SchemaInfo;
use super::FormatType;
use crate::error::{RenoirError, Result};
use std::collections::HashMap;

/// Core trait for zero-copy message format implementations
pub trait ZeroCopyFormat: Send + Sync {
    /// Get the format type identifier
    fn format_type(&self) -> FormatType;

    /// Get format name for debugging
    fn name(&self) -> &'static str {
        self.format_type().name()
    }

    /// Validate schema compatibility
    fn validate_schema(&self, schema: &SchemaInfo) -> Result<()> {
        if schema.format_type != self.format_type() {
            return Err(RenoirError::invalid_parameter(
                "schema",
                &format!(
                    "Schema format {} doesn't match format {}",
                    schema.format_type.name(),
                    self.name()
                ),
            ));
        }
        Ok(())
    }

    /// Create a zero-copy buffer from raw data
    fn create_buffer_from_bytes(&self, data: &[u8], schema: &SchemaInfo) -> Result<Vec<u8>>;

    /// Create zero-copy accessor for existing buffer
    fn create_accessor(&self, bytes: &[u8], schema: &SchemaInfo) -> Result<ZeroCopyAccessor>;
}

/// Zero-copy accessor that avoids trait object issues
#[derive(Debug)]
pub struct ZeroCopyAccessor {
    buffer: Vec<u8>,
    schema: SchemaInfo,
    format_type: FormatType,
    field_offsets: HashMap<String, (usize, FieldType)>,
}

impl ZeroCopyAccessor {
    /// Create new accessor
    pub fn new(
        buffer: Vec<u8>,
        schema: SchemaInfo,
        field_offsets: HashMap<String, (usize, FieldType)>,
    ) -> Self {
        Self {
            format_type: schema.format_type,
            buffer,
            schema,
            field_offsets,
        }
    }

    /// Get the underlying buffer
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Get schema information
    pub fn schema(&self) -> &SchemaInfo {
        &self.schema
    }

    /// Get format type
    pub fn format_type(&self) -> FormatType {
        self.format_type
    }

    /// Get a field value by name and type
    pub fn get_field_bytes(&self, field_name: &str) -> Result<Option<&[u8]>> {
        if let Some((offset, field_type)) = self.field_offsets.get(field_name) {
            let size = field_type.size();
            if *offset + size <= self.buffer.len() {
                Ok(Some(&self.buffer[*offset..*offset + size]))
            } else {
                Err(RenoirError::invalid_parameter(
                    "field_offset",
                    "Field offset exceeds buffer bounds",
                ))
            }
        } else {
            Ok(None)
        }
    }

    /// Get a u32 field value
    pub fn get_u32(&self, field_name: &str) -> Result<Option<u32>> {
        if let Some(bytes) = self.get_field_bytes(field_name)? {
            if bytes.len() >= 4 {
                Ok(Some(u32::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3],
                ])))
            } else {
                Err(RenoirError::invalid_parameter(
                    "field_size",
                    "Field too small for u32",
                ))
            }
        } else {
            Ok(None)
        }
    }

    /// Get a u64 field value
    pub fn get_u64(&self, field_name: &str) -> Result<Option<u64>> {
        if let Some(bytes) = self.get_field_bytes(field_name)? {
            if bytes.len() >= 8 {
                Ok(Some(u64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ])))
            } else {
                Err(RenoirError::invalid_parameter(
                    "field_size",
                    "Field too small for u64",
                ))
            }
        } else {
            Ok(None)
        }
    }

    /// Get a f64 field value
    pub fn get_f64(&self, field_name: &str) -> Result<Option<f64>> {
        if let Some(bytes) = self.get_field_bytes(field_name)? {
            if bytes.len() >= 8 {
                Ok(Some(f64::from_le_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ])))
            } else {
                Err(RenoirError::invalid_parameter(
                    "field_size",
                    "Field too small for f64",
                ))
            }
        } else {
            Ok(None)
        }
    }

    /// Get a string field value
    pub fn get_string(&self, field_name: &str) -> Result<Option<&str>> {
        if let Some(bytes) = self.get_field_bytes(field_name)? {
            std::str::from_utf8(bytes)
                .map(Some)
                .map_err(|_| RenoirError::invalid_parameter("field_encoding", "Invalid UTF-8"))
        } else {
            Ok(None)
        }
    }

    /// Get a byte array field
    pub fn get_bytes(&self, field_name: &str) -> Result<Option<&[u8]>> {
        self.get_field_bytes(field_name)
    }

    /// List all available fields
    pub fn list_fields(&self) -> Vec<&str> {
        self.field_offsets.keys().map(|s| s.as_str()).collect()
    }
}

/// Field type information for zero-copy access
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    String(usize), // String with length
    Bytes(usize),  // Byte array with length
}

impl FieldType {
    /// Get the size in bytes of this field type
    pub fn size(&self) -> usize {
        match self {
            FieldType::U8 | FieldType::I8 => 1,
            FieldType::U16 | FieldType::I16 => 2,
            FieldType::U32 | FieldType::I32 | FieldType::F32 => 4,
            FieldType::U64 | FieldType::I64 | FieldType::F64 => 8,
            FieldType::String(len) | FieldType::Bytes(len) => *len,
        }
    }
}

/// Builder for zero-copy buffers
pub struct ZeroCopyBuilder {
    format_type: FormatType,
    buffer: Vec<u8>,
    field_offsets: HashMap<String, (usize, FieldType)>,
    current_offset: usize,
}

impl ZeroCopyBuilder {
    /// Create new builder
    pub fn new(format_type: FormatType) -> Self {
        Self {
            format_type,
            buffer: Vec::new(),
            field_offsets: HashMap::new(),
            current_offset: 0,
        }
    }

    /// Get the format type of this builder
    pub fn format_type(&self) -> FormatType {
        self.format_type
    }

    /// Add a u32 field
    pub fn add_u32(&mut self, field_name: String, value: u32) -> Result<()> {
        let bytes = value.to_le_bytes();
        self.field_offsets
            .insert(field_name, (self.current_offset, FieldType::U32));
        self.buffer.extend_from_slice(&bytes);
        self.current_offset += 4;
        Ok(())
    }

    /// Add a u64 field
    pub fn add_u64(&mut self, field_name: String, value: u64) -> Result<()> {
        let bytes = value.to_le_bytes();
        self.field_offsets
            .insert(field_name, (self.current_offset, FieldType::U64));
        self.buffer.extend_from_slice(&bytes);
        self.current_offset += 8;
        Ok(())
    }

    /// Add a f64 field
    pub fn add_f64(&mut self, field_name: String, value: f64) -> Result<()> {
        let bytes = value.to_le_bytes();
        self.field_offsets
            .insert(field_name, (self.current_offset, FieldType::F64));
        self.buffer.extend_from_slice(&bytes);
        self.current_offset += 8;
        Ok(())
    }

    /// Add a string field
    pub fn add_string(&mut self, field_name: String, value: &str) -> Result<()> {
        let bytes = value.as_bytes();
        self.field_offsets.insert(
            field_name,
            (self.current_offset, FieldType::String(bytes.len())),
        );
        self.buffer.extend_from_slice(bytes);
        self.current_offset += bytes.len();
        Ok(())
    }

    /// Add a byte array field
    pub fn add_bytes(&mut self, field_name: String, value: &[u8]) -> Result<()> {
        self.field_offsets.insert(
            field_name,
            (self.current_offset, FieldType::Bytes(value.len())),
        );
        self.buffer.extend_from_slice(value);
        self.current_offset += value.len();
        Ok(())
    }

    /// Finish building and return the accessor
    pub fn finish(self, schema: SchemaInfo) -> ZeroCopyAccessor {
        ZeroCopyAccessor::new(self.buffer, schema, self.field_offsets)
    }

    /// Get the current buffer (for debugging)
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }
}

/// Schema validation trait
pub trait SchemaValidator: Send + Sync {
    /// Validate that a buffer matches the expected schema
    fn validate_buffer(&self, buffer: &[u8], expected_schema: &SchemaInfo) -> Result<()>;

    /// Check if two schemas are compatible for message exchange
    fn check_compatibility(
        &self,
        producer_schema: &SchemaInfo,
        consumer_schema: &SchemaInfo,
    ) -> Result<SchemaCompatibility>;

    /// Extract schema information from a buffer
    fn extract_schema_info(&self, buffer: &[u8]) -> Result<SchemaInfo>;
}

/// Schema compatibility result
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaCompatibility {
    /// Schemas are identical
    Identical,
    /// Schemas are compatible (forward/backward compatible)
    Compatible,
    /// Schema upgrade is needed but possible
    UpgradeNeeded,
    /// Schemas are incompatible
    Incompatible,
}

impl SchemaCompatibility {
    /// Check if the schemas can be used together
    pub fn is_compatible(&self) -> bool {
        matches!(
            self,
            SchemaCompatibility::Identical | SchemaCompatibility::Compatible
        )
    }

    /// Check if an upgrade is needed
    pub fn needs_upgrade(&self) -> bool {
        matches!(self, SchemaCompatibility::UpgradeNeeded)
    }
}

// Re-export for compatibility
pub use ZeroCopyAccessor as ZeroCopyAccess;
pub use ZeroCopyBuilder as BufferBuilder;
