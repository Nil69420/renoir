//! Zero-copy message format registry
//!
//! Provides centralized management of zero-copy serialization formats and schemas

use super::schema_evolution::{CompatibilityLevel, EvolutionAwareSchema, SchemaEvolutionManager};
use super::traits::ZeroCopyFormat;
use super::zero_copy::{CapnProtoFormat, FlatBufferFormat};
use super::FormatType;
use crate::error::{RenoirError, Result};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// Registry for managing zero-copy message formats and schemas
pub struct ZeroCopyFormatRegistry {
    /// Registered formats by type
    formats: HashMap<FormatType, Arc<dyn ZeroCopyFormat + Send + Sync>>,
    /// Registered schemas by name and version
    schemas: HashMap<String, Vec<SchemaInfo>>,
    /// Topic format assignments
    topic_formats: HashMap<String, FormatType>,
    /// Topic schema assignments
    topic_schemas: HashMap<String, SchemaInfo>,
    /// Schema evolution management
    evolution_manager: SchemaEvolutionManager,
    /// Enhanced compatibility tracking
    compatibility_cache: HashMap<(u64, u32, u32), CompatibilityLevel>,
}

impl Default for ZeroCopyFormatRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ZeroCopyFormatRegistry {
    /// Create new registry with default zero-copy formats
    pub fn new() -> Self {
        let mut registry = Self {
            formats: HashMap::new(),
            schemas: HashMap::new(),
            topic_formats: HashMap::new(),
            topic_schemas: HashMap::new(),
            evolution_manager: SchemaEvolutionManager::new(),
            compatibility_cache: HashMap::new(),
        };

        // Register default zero-copy formats
        registry.register_default_formats();

        registry
    }

    /// Register all default zero-copy message formats
    fn register_default_formats(&mut self) {
        // Zero-copy formats only
        self.formats
            .insert(FormatType::FlatBuffers, Arc::new(FlatBufferFormat::new()));
        self.formats
            .insert(FormatType::CapnProto, Arc::new(CapnProtoFormat::new()));
    }

    /// Register a custom zero-copy format
    pub fn register_format(
        &mut self,
        format_type: FormatType,
        format: Arc<dyn ZeroCopyFormat + Send + Sync>,
    ) -> Result<()> {
        if self.formats.contains_key(&format_type) {
            return Err(RenoirError::invalid_parameter(
                "format_type",
                &format!("Format {} already registered", format_type.name()),
            ));
        }

        self.formats.insert(format_type, format);
        Ok(())
    }

    /// Get a format by type
    pub fn get_format(
        &self,
        format_type: &FormatType,
    ) -> Option<Arc<dyn ZeroCopyFormat + Send + Sync>> {
        self.formats.get(format_type).cloned()
    }

    /// Register a schema for a specific format
    pub fn register_schema(&mut self, schema: SchemaInfo) -> Result<()> {
        if !self.formats.contains_key(&schema.format_type) {
            return Err(RenoirError::invalid_parameter(
                "schema",
                &format!("Format {} not registered", schema.format_type.name()),
            ));
        }

        self.schemas
            .entry(schema.schema_name.clone())
            .or_insert_with(Vec::new)
            .push(schema);

        Ok(())
    }

    /// Get the latest schema version for a name
    pub fn get_latest_schema(&self, schema_name: &str) -> Option<&SchemaInfo> {
        self.schemas
            .get(schema_name)?
            .iter()
            .max_by_key(|s| s.schema_version)
    }

    /// Get a specific schema version
    pub fn get_schema(&self, schema_name: &str, version: u32) -> Option<&SchemaInfo> {
        self.schemas
            .get(schema_name)?
            .iter()
            .find(|s| s.schema_version == version)
    }

    /// Assign a format to a topic
    pub fn assign_topic_format(
        &mut self,
        topic_pattern: String,
        format_type: FormatType,
    ) -> Result<()> {
        if !self.formats.contains_key(&format_type) {
            return Err(RenoirError::invalid_parameter(
                "format_type",
                &format!("Format {} not registered", format_type.name()),
            ));
        }

        self.topic_formats.insert(topic_pattern, format_type);
        Ok(())
    }

    /// Assign a schema to a topic
    pub fn assign_topic_schema(&mut self, topic_pattern: String, schema: SchemaInfo) -> Result<()> {
        self.register_schema(schema.clone())?;
        self.topic_schemas.insert(topic_pattern, schema);
        Ok(())
    }

    /// Get the assigned format for a topic
    pub fn get_topic_format(&self, topic_name: &str) -> Option<FormatType> {
        // First try exact match
        if let Some(format) = self.topic_formats.get(topic_name) {
            return Some(*format);
        }

        // Then try pattern matching
        for (pattern, format) in &self.topic_formats {
            if topic_matches_pattern(topic_name, pattern) {
                return Some(*format);
            }
        }

        None
    }

    /// Get the assigned schema for a topic
    pub fn get_topic_schema(&self, topic_name: &str) -> Option<&SchemaInfo> {
        // First try exact match
        if let Some(schema) = self.topic_schemas.get(topic_name) {
            return Some(schema);
        }

        // Then try pattern matching
        for (pattern, schema) in &self.topic_schemas {
            if topic_matches_pattern(topic_name, pattern) {
                return Some(schema);
            }
        }

        None
    }

    /// Get format recommendations based on use case
    pub fn recommend_format(&self, use_case: UseCase) -> FormatType {
        match use_case {
            UseCase::HighFrequency => FormatType::FlatBuffers, // Best performance
            UseCase::LowLatency => FormatType::FlatBuffers,    // Lowest latency
            UseCase::LargeMessages => FormatType::CapnProto,   // Better for large data
            UseCase::CrossLanguage => FormatType::FlatBuffers, // Wider support
            UseCase::RealTime => FormatType::FlatBuffers,      // Deterministic
            UseCase::Streaming => FormatType::CapnProto,       // Stream-friendly
            UseCase::GeneralPurpose => FormatType::FlatBuffers, // Good default
        }
    }

    /// Register an evolution-aware schema
    pub fn register_evolution_schema(&mut self, schema: EvolutionAwareSchema) -> Result<()> {
        // Register with evolution manager
        self.evolution_manager.register_schema(schema.clone())?;

        // Also register in legacy system for compatibility
        self.register_schema(schema.base)?;

        Ok(())
    }

    /// Check compatibility between two schemas using evolution rules
    pub fn check_schema_compatibility(
        &mut self,
        reader_name: &str,
        writer_name: &str,
    ) -> Result<CompatibilityLevel> {
        let reader_schema = self
            .evolution_manager
            .get_latest_schema(reader_name)
            .ok_or_else(|| RenoirError::invalid_parameter("reader_name", "Schema not found"))?;

        let writer_schema = self
            .evolution_manager
            .get_latest_schema(writer_name)
            .ok_or_else(|| RenoirError::invalid_parameter("writer_name", "Schema not found"))?;

        // Check cache first
        let cache_key = (
            reader_schema.schema_id,
            reader_schema.base.schema_version,
            writer_schema.base.schema_version,
        );
        if let Some(cached) = self.compatibility_cache.get(&cache_key) {
            return Ok(*cached);
        }

        // Compute compatibility
        let compatibility = self
            .evolution_manager
            .check_compatibility(reader_schema, writer_schema)?;

        // Cache result
        self.compatibility_cache.insert(cache_key, compatibility);

        Ok(compatibility)
    }

    /// Get migration plan for schema evolution
    pub fn get_migration_plan(
        &self,
        from_schema_name: &str,
        to_schema_name: &str,
    ) -> Result<Vec<super::schema_evolution::MigrationStep>> {
        let from_schema = self
            .evolution_manager
            .get_latest_schema(from_schema_name)
            .ok_or_else(|| RenoirError::invalid_parameter("from_schema", "Schema not found"))?;

        let to_schema = self
            .evolution_manager
            .get_latest_schema(to_schema_name)
            .ok_or_else(|| RenoirError::invalid_parameter("to_schema", "Schema not found"))?;

        self.evolution_manager
            .generate_migration_plan(from_schema, to_schema)
    }

    /// Validate backward compatibility for a schema update
    pub fn validate_backward_compatibility(
        &mut self,
        old_name: &str,
        new_schema: &EvolutionAwareSchema,
    ) -> Result<bool> {
        let old_schema = self.evolution_manager.get_latest_schema(old_name);

        if let Some(old) = old_schema {
            let compatibility = self
                .evolution_manager
                .check_compatibility(old, new_schema)?;
            Ok(matches!(
                compatibility,
                CompatibilityLevel::FullCompatible | CompatibilityLevel::BackwardCompatible
            ))
        } else {
            // No previous version - always compatible
            Ok(true)
        }
    }

    /// Clear compatibility cache (useful after schema updates)
    pub fn clear_compatibility_cache(&mut self) {
        self.compatibility_cache.clear();
    }

    /// Generate a unique schema ID
    pub fn generate_schema_id(&mut self) -> u64 {
        self.evolution_manager.generate_schema_id()
    }

    /// List all registered formats
    pub fn list_formats(&self) -> Vec<FormatType> {
        self.formats.keys().cloned().collect()
    }

    /// List all registered schemas
    pub fn list_schemas(&self) -> Vec<&SchemaInfo> {
        self.schemas.values().flatten().collect()
    }
}

/// Use case categories for format recommendation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseCase {
    /// High-frequency message publishing (>1kHz)
    HighFrequency,
    /// Low-latency requirements (<1ms)
    LowLatency,
    /// Large message payloads (>1MB)
    LargeMessages,
    /// Cross-language compatibility needed
    CrossLanguage,
    /// Real-time system requirements
    RealTime,
    /// Streaming data applications
    Streaming,
    /// General purpose usage
    GeneralPurpose,
}

/// Schema metadata that can be made public
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaInfo {
    pub format_type: FormatType,
    pub schema_name: String,
    pub schema_version: u32,
    pub schema_hash: u64,
}

impl SchemaInfo {
    pub fn new(
        format_type: FormatType,
        schema_name: String,
        schema_version: u32,
        schema_hash: u64,
    ) -> Self {
        Self {
            format_type,
            schema_name,
            schema_version,
            schema_hash,
        }
    }

    /// Check if two schemas are compatible
    pub fn is_compatible(&self, other: &SchemaInfo) -> bool {
        self.format_type == other.format_type
            && self.schema_name == other.schema_name
            && self.schema_hash == other.schema_hash
    }
}

/// Global registry instance
static GLOBAL_REGISTRY: OnceLock<RwLock<ZeroCopyFormatRegistry>> = OnceLock::new();

/// Initialize the global registry
pub fn init_global_registry() {
    let _ = GLOBAL_REGISTRY.set(RwLock::new(ZeroCopyFormatRegistry::new()));
}

/// Get the global registry (read-only)
pub fn global_registry() -> &'static RwLock<ZeroCopyFormatRegistry> {
    GLOBAL_REGISTRY.get_or_init(|| RwLock::new(ZeroCopyFormatRegistry::new()))
}

/// Register a schema globally
pub fn register_global_schema(schema: SchemaInfo) -> Result<()> {
    let registry = global_registry();
    let mut registry = registry
        .write()
        .map_err(|_| RenoirError::memory("Registry lock poisoned".to_string()))?;
    registry.register_schema(schema)
}

/// Get a format from the global registry
pub fn get_global_format(
    format_type: &FormatType,
) -> Option<Arc<dyn ZeroCopyFormat + Send + Sync>> {
    let registry = global_registry();
    let registry = registry.read().ok()?;
    registry.get_format(format_type)
}

/// Get format recommendation from global registry
pub fn get_global_recommendation(use_case: &UseCase) -> FormatType {
    let registry = global_registry();
    let registry = registry.read().expect("Registry lock poisoned");
    registry.recommend_format(*use_case)
}

/// Topic pattern matching helper
fn topic_matches_pattern(topic_name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if pattern.ends_with("/*") {
        let prefix = &pattern[..pattern.len() - 2];
        return topic_name.starts_with(prefix);
    }

    if pattern.starts_with("*/") {
        let suffix = &pattern[2..];
        return topic_name.ends_with(suffix);
    }

    // Exact match
    topic_name == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_pattern_matching() {
        assert!(topic_matches_pattern("sensor/imu", "*"));
        assert!(topic_matches_pattern("sensor/imu", "sensor/*"));
        assert!(topic_matches_pattern("sensor/imu", "*/imu"));
        assert!(topic_matches_pattern("sensor/imu", "sensor/imu"));
        assert!(!topic_matches_pattern("sensor/imu", "control/*"));
    }

    #[test]
    fn test_format_registration() {
        let registry = ZeroCopyFormatRegistry::new();

        // Should have default formats
        assert!(registry.get_format(&FormatType::FlatBuffers).is_some());
        assert!(registry.get_format(&FormatType::CapnProto).is_some());
    }

    #[test]
    fn test_schema_registration() {
        let mut registry = ZeroCopyFormatRegistry::new();

        let schema = SchemaInfo::new(FormatType::FlatBuffers, "test_schema".to_string(), 1, 12345);

        registry.register_schema(schema.clone()).unwrap();

        let retrieved = registry.get_latest_schema("test_schema").unwrap();
        assert_eq!(retrieved.schema_version, 1);
        assert_eq!(retrieved.schema_name, "test_schema");
    }
}
