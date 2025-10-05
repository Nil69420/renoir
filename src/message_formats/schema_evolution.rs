//! Schema Evolution Strategy for Zero-Copy Message Formats
//!
//! This module provides comprehensive schema evolution support with:
//! - Schema ID system with version tracking
//! - Backward/forward compatibility rules for FlatBuffers  
//! - Migration framework for breaking changes
//! - Automated compatibility validation

use super::{registry::SchemaInfo, FormatType};
use crate::error::{RenoirError, Result};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

/// Schema evolution compatibility levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompatibilityLevel {
    /// Full backward and forward compatibility
    FullCompatible,
    /// Backward compatible only (older readers can read newer schemas)
    BackwardCompatible,
    /// Forward compatible only (newer readers can read older schemas)
    ForwardCompatible,
    /// No compatibility - breaking change requiring migration
    Breaking,
}

/// Schema evolution rules and validation
#[derive(Debug, Clone)]
pub struct SchemaEvolutionRule {
    /// The compatibility level between schema versions
    pub compatibility: CompatibilityLevel,
    /// Field changes that determine compatibility
    pub field_changes: Vec<FieldChange>,
    /// Required migration steps for breaking changes
    pub migration_steps: Vec<MigrationStep>,
}

/// Types of field changes in schema evolution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldChange {
    /// New optional field added (backward compatible)
    OptionalFieldAdded {
        field_name: String,
        field_type: String,
    },
    /// Required field added (breaking change)
    RequiredFieldAdded {
        field_name: String,
        field_type: String,
    },
    /// Field removed (potentially breaking)
    FieldRemoved {
        field_name: String,
        was_required: bool,
    },
    /// Field type changed (breaking if incompatible)
    FieldTypeChanged {
        field_name: String,
        old_type: String,
        new_type: String,
        is_compatible: bool,
    },
    /// Field made optional (forward compatible)
    FieldMadeOptional { field_name: String },
    /// Field made required (breaking change)
    FieldMadeRequired { field_name: String },
    /// Field deprecated (backward compatible)
    FieldDeprecated { field_name: String },
    /// Field order changed (format-specific compatibility)
    FieldReordered {
        field_name: String,
        old_position: u32,
        new_position: u32,
    },
}

/// Migration step for handling breaking changes
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationStep {
    /// Default value assignment for new required fields
    AssignDefaultValue {
        field_name: String,
        default_value: String,
    },
    /// Field mapping from old to new structure
    MapField {
        old_field: String,
        new_field: String,
        transform: String,
    },
    /// Custom migration function call
    CustomTransform {
        function_name: String,
        parameters: Vec<String>,
    },
    /// Data validation step
    ValidateData { validation_rule: String },
    /// Schema version bump
    VersionBump {
        from_version: u32,
        to_version: u32,
        is_major: bool,
    },
}

/// Enhanced schema information with evolution metadata
#[derive(Debug, Clone)]
pub struct EvolutionAwareSchema {
    /// Base schema information
    pub base: SchemaInfo,
    /// Schema ID for tracking across versions
    pub schema_id: u64,
    /// Semantic version (major.minor.patch)
    pub semantic_version: SemanticVersion,
    /// Compatibility rules with other versions
    pub compatibility_rules: BTreeMap<u32, SchemaEvolutionRule>,
    /// Field definitions and metadata
    pub fields: HashMap<String, FieldDefinition>,
    /// Deprecated fields still supported for backward compatibility
    pub deprecated_fields: HashSet<String>,
    /// Required migration version (if any)
    pub requires_migration_from: Option<u32>,
}

/// Semantic versioning for schemas
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemanticVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// Field definition with evolution metadata
#[derive(Debug, Clone)]
pub struct FieldDefinition {
    pub field_name: String,
    pub field_type: String,
    pub is_required: bool,
    pub is_deprecated: bool,
    pub added_in_version: u32,
    pub deprecated_in_version: Option<u32>,
    pub default_value: Option<String>,
    pub position: u32,
}

/// Schema evolution manager
pub struct SchemaEvolutionManager {
    /// All registered schemas by ID
    schemas: HashMap<u64, Vec<EvolutionAwareSchema>>,
    /// Schema name to ID mapping
    name_to_id: HashMap<String, u64>,
    /// Next available schema ID
    next_schema_id: u64,
}

impl SchemaEvolutionManager {
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
            name_to_id: HashMap::new(),
            next_schema_id: 1,
        }
    }

    /// Register a new schema with evolution support
    pub fn register_schema(&mut self, schema: EvolutionAwareSchema) -> Result<()> {
        let schema_id = schema.schema_id;
        let name = schema.base.schema_name.clone();

        // Validate schema evolution rules
        self.validate_schema_evolution(&schema)?;

        // Store schema
        self.schemas
            .entry(schema_id)
            .or_insert_with(Vec::new)
            .push(schema);

        // Update name mapping
        self.name_to_id.insert(name, schema_id);

        Ok(())
    }

    /// Create a new schema ID
    pub fn generate_schema_id(&mut self) -> u64 {
        let id = self.next_schema_id;
        self.next_schema_id += 1;
        id
    }

    /// Get the latest version of a schema by name
    pub fn get_latest_schema(&self, name: &str) -> Option<&EvolutionAwareSchema> {
        let schema_id = self.name_to_id.get(name)?;
        let versions = self.schemas.get(schema_id)?;
        versions.iter().max_by_key(|s| &s.semantic_version)
    }

    /// Get a specific schema version
    pub fn get_schema_version(
        &self,
        name: &str,
        version: &SemanticVersion,
    ) -> Option<&EvolutionAwareSchema> {
        let schema_id = self.name_to_id.get(name)?;
        let versions = self.schemas.get(schema_id)?;
        versions.iter().find(|s| s.semantic_version == *version)
    }

    /// Check compatibility between two schema versions
    pub fn check_compatibility(
        &self,
        reader_schema: &EvolutionAwareSchema,
        writer_schema: &EvolutionAwareSchema,
    ) -> Result<CompatibilityLevel> {
        // Same schema is always compatible
        if reader_schema.schema_id == writer_schema.schema_id
            && reader_schema.semantic_version == writer_schema.semantic_version
        {
            return Ok(CompatibilityLevel::FullCompatible);
        }

        // Different schema names/IDs are incompatible
        if reader_schema.schema_id != writer_schema.schema_id {
            return Ok(CompatibilityLevel::Breaking);
        }

        // Check semantic version compatibility
        self.check_semantic_version_compatibility(
            &reader_schema.semantic_version,
            &writer_schema.semantic_version,
        )
    }

    /// Generate migration plan for breaking schema changes
    pub fn generate_migration_plan(
        &self,
        from_schema: &EvolutionAwareSchema,
        to_schema: &EvolutionAwareSchema,
    ) -> Result<Vec<MigrationStep>> {
        if from_schema.schema_id != to_schema.schema_id {
            return Err(RenoirError::invalid_parameter(
                "schema",
                "Cannot migrate between different schema types",
            ));
        }

        let compatibility = self.check_compatibility(from_schema, to_schema)?;

        match compatibility {
            CompatibilityLevel::FullCompatible
            | CompatibilityLevel::BackwardCompatible
            | CompatibilityLevel::ForwardCompatible => {
                // No migration needed
                Ok(vec![])
            }
            CompatibilityLevel::Breaking => {
                // Generate migration steps based on field differences
                self.generate_breaking_change_migration(from_schema, to_schema)
            }
        }
    }

    /// Validate that evolution rules are consistent and valid
    fn validate_schema_evolution(&self, schema: &EvolutionAwareSchema) -> Result<()> {
        // Check that all field positions are unique
        let mut positions = HashSet::new();
        for field in schema.fields.values() {
            if !positions.insert(field.position) {
                return Err(RenoirError::invalid_parameter(
                    "schema",
                    &format!("Duplicate field position {} in schema", field.position),
                ));
            }
        }

        // Validate compatibility rules
        for (version, rule) in &schema.compatibility_rules {
            self.validate_evolution_rule(schema, *version, rule)?;
        }

        Ok(())
    }

    fn validate_evolution_rule(
        &self,
        _schema: &EvolutionAwareSchema,
        _version: u32,
        rule: &SchemaEvolutionRule,
    ) -> Result<()> {
        // Validate that the evolution rule is well-formed
        // Check if the rule is compatible based on its field changes
        for change in &rule.field_changes {
            match change {
                FieldChange::OptionalFieldAdded { .. } => {
                    // Always compatible - readers can ignore unknown fields
                    continue;
                }
                FieldChange::RequiredFieldAdded { field_name, .. } => {
                    // Breaking - old readers cannot handle required fields
                    if !matches!(rule.compatibility, CompatibilityLevel::Breaking) {
                        return Err(RenoirError::invalid_parameter(
                            "rule",
                            &format!("Required field '{}' added but not marked as breaking", field_name),
                        ));
                    }
                }
                FieldChange::FieldRemoved { field_name, was_required } => {
                    // Breaking if required, potentially compatible if optional
                    if *was_required && !matches!(rule.compatibility, CompatibilityLevel::Breaking) {
                        return Err(RenoirError::invalid_parameter(
                            "rule",
                            &format!("Required field '{}' removed but not marked as breaking", field_name),
                        ));
                    }
                }
                FieldChange::FieldTypeChanged { field_name, is_compatible, .. } => {
                    // Breaking if incompatible type change
                    if !is_compatible && !matches!(rule.compatibility, CompatibilityLevel::Breaking) {
                        return Err(RenoirError::invalid_parameter(
                            "rule",
                            &format!("Incompatible type change for '{}' but not marked as breaking", field_name),
                        ));
                    }
                }
                FieldChange::FieldMadeRequired { field_name } => {
                    // Always breaking - old data may not have values
                    if !matches!(rule.compatibility, CompatibilityLevel::Breaking) {
                        return Err(RenoirError::invalid_parameter(
                            "rule",
                            &format!("Field '{}' made required but not marked as breaking", field_name),
                        ));
                    }
                }
                FieldChange::FieldMadeOptional { .. } | 
                FieldChange::FieldDeprecated { .. } | 
                FieldChange::FieldReordered { .. } => {
                    // Generally compatible changes
                    continue;
                }
            }
        }

        // If marked as breaking, ensure migration steps exist
        if matches!(rule.compatibility, CompatibilityLevel::Breaking) && rule.migration_steps.is_empty() {
            return Err(RenoirError::invalid_parameter(
                "rule",
                "Breaking change requires migration steps",
            ));
        }

        Ok(())
    }

    fn check_semantic_version_compatibility(
        &self,
        reader_version: &SemanticVersion,
        writer_version: &SemanticVersion,
    ) -> Result<CompatibilityLevel> {
        // Major version differences are breaking changes
        if reader_version.major != writer_version.major {
            return Ok(CompatibilityLevel::Breaking);
        }

        // Minor version increases are backward compatible
        match reader_version.minor.cmp(&writer_version.minor) {
            std::cmp::Ordering::Less => Ok(CompatibilityLevel::BackwardCompatible),
            std::cmp::Ordering::Greater => Ok(CompatibilityLevel::ForwardCompatible),
            std::cmp::Ordering::Equal => {
                // Same minor version, check patch level
                if reader_version.patch == writer_version.patch {
                    Ok(CompatibilityLevel::FullCompatible)
                } else {
                    // Patch differences are fully compatible
                    Ok(CompatibilityLevel::FullCompatible)
                }
            }
        }
    }

    fn generate_breaking_change_migration(
        &self,
        from_schema: &EvolutionAwareSchema,
        to_schema: &EvolutionAwareSchema,
    ) -> Result<Vec<MigrationStep>> {
        let mut steps = vec![MigrationStep::VersionBump {
            from_version: from_schema.base.schema_version,
            to_version: to_schema.base.schema_version,
            is_major: from_schema.semantic_version.major != to_schema.semantic_version.major,
        }];

        // Compare fields to determine migration steps
        for (field_name, new_field) in &to_schema.fields {
            if let Some(old_field) = from_schema.fields.get(field_name) {
                // Field exists in both - check for changes
                if old_field.field_type != new_field.field_type {
                    steps.push(MigrationStep::MapField {
                        old_field: field_name.clone(),
                        new_field: field_name.clone(),
                        transform: format!(
                            "convert_{}_to_{}",
                            old_field.field_type, new_field.field_type
                        ),
                    });
                }

                if !old_field.is_required && new_field.is_required {
                    if let Some(default) = &new_field.default_value {
                        steps.push(MigrationStep::AssignDefaultValue {
                            field_name: field_name.clone(),
                            default_value: default.clone(),
                        });
                    }
                }
            } else {
                // New field - assign default if required
                if new_field.is_required {
                    if let Some(default) = &new_field.default_value {
                        steps.push(MigrationStep::AssignDefaultValue {
                            field_name: field_name.clone(),
                            default_value: default.clone(),
                        });
                    } else {
                        return Err(RenoirError::invalid_parameter(
                            "schema",
                            &format!("New required field '{}' needs a default value", field_name),
                        ));
                    }
                }
            }
        }

        // Add validation step
        steps.push(MigrationStep::ValidateData {
            validation_rule: format!(
                "validate_migration_{}_{}",
                from_schema.semantic_version, to_schema.semantic_version
            ),
        });

        Ok(steps)
    }
}

impl Default for SchemaEvolutionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Check if this version is compatible with another version
    pub fn is_compatible_with(&self, other: &SemanticVersion) -> CompatibilityLevel {
        if self.major != other.major {
            CompatibilityLevel::Breaking
        } else if self.minor < other.minor {
            CompatibilityLevel::BackwardCompatible
        } else if self.minor > other.minor {
            CompatibilityLevel::ForwardCompatible
        } else {
            CompatibilityLevel::FullCompatible
        }
    }
}

impl fmt::Display for SemanticVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FieldDefinition {
    pub fn new(
        field_name: String,
        field_type: String,
        is_required: bool,
        added_in_version: u32,
        position: u32,
    ) -> Self {
        Self {
            field_name,
            field_type,
            is_required,
            is_deprecated: false,
            added_in_version,
            deprecated_in_version: None,
            default_value: None,
            position,
        }
    }

    pub fn with_default(mut self, default_value: String) -> Self {
        self.default_value = Some(default_value);
        self
    }

    pub fn deprecated_in(mut self, version: u32) -> Self {
        self.is_deprecated = true;
        self.deprecated_in_version = Some(version);
        self
    }
}

/// Builder for creating evolution-aware schemas
pub struct SchemaBuilder {
    schema_name: String,
    schema_id: Option<u64>,
    format_type: FormatType,
    semantic_version: SemanticVersion,
    fields: HashMap<String, FieldDefinition>,
    compatibility_rules: BTreeMap<u32, SchemaEvolutionRule>,
}

impl SchemaBuilder {
    pub fn new(schema_name: String, format_type: FormatType) -> Self {
        Self {
            schema_name,
            schema_id: None,
            format_type,
            semantic_version: SemanticVersion::new(1, 0, 0),
            fields: HashMap::new(),
            compatibility_rules: BTreeMap::new(),
        }
    }

    pub fn with_id(mut self, schema_id: u64) -> Self {
        self.schema_id = Some(schema_id);
        self
    }

    pub fn version(mut self, major: u32, minor: u32, patch: u32) -> Self {
        self.semantic_version = SemanticVersion::new(major, minor, patch);
        self
    }

    pub fn add_field(mut self, field: FieldDefinition) -> Self {
        self.fields.insert(field.field_name.clone(), field);
        self
    }

    pub fn add_compatibility_rule(mut self, version: u32, rule: SchemaEvolutionRule) -> Self {
        self.compatibility_rules.insert(version, rule);
        self
    }

    pub fn build(self, manager: &mut SchemaEvolutionManager) -> Result<EvolutionAwareSchema> {
        let schema_id = self
            .schema_id
            .unwrap_or_else(|| manager.generate_schema_id());
        let schema_version = self.semantic_version.major * 10000
            + self.semantic_version.minor * 100
            + self.semantic_version.patch;

        let base = SchemaInfo::new(
            self.format_type,
            self.schema_name.clone(),
            schema_version,
            schema_id, // Using schema_id as hash for now
        );

        Ok(EvolutionAwareSchema {
            base,
            schema_id,
            semantic_version: self.semantic_version,
            compatibility_rules: self.compatibility_rules,
            fields: self.fields,
            deprecated_fields: HashSet::new(),
            requires_migration_from: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_version_compatibility() {
        let v1_0_0 = SemanticVersion::new(1, 0, 0);
        let v1_1_0 = SemanticVersion::new(1, 1, 0);
        let v1_0_1 = SemanticVersion::new(1, 0, 1);
        let v2_0_0 = SemanticVersion::new(2, 0, 0);

        assert_eq!(
            v1_0_0.is_compatible_with(&v1_0_0),
            CompatibilityLevel::FullCompatible
        );
        assert_eq!(
            v1_0_0.is_compatible_with(&v1_1_0),
            CompatibilityLevel::BackwardCompatible
        );
        assert_eq!(
            v1_1_0.is_compatible_with(&v1_0_0),
            CompatibilityLevel::ForwardCompatible
        );
        assert_eq!(
            v1_0_0.is_compatible_with(&v2_0_0),
            CompatibilityLevel::Breaking
        );
        assert_eq!(
            v1_0_0.is_compatible_with(&v1_0_1),
            CompatibilityLevel::FullCompatible
        );
    }

    #[test]
    fn test_schema_evolution_manager() -> Result<()> {
        let mut manager = SchemaEvolutionManager::new();

        let schema = SchemaBuilder::new("test_schema".to_string(), FormatType::FlatBuffers)
            .version(1, 0, 0)
            .add_field(FieldDefinition::new(
                "field1".to_string(),
                "u32".to_string(),
                true,
                1,
                0,
            ))
            .build(&mut manager)?;

        manager.register_schema(schema)?;

        let retrieved = manager.get_latest_schema("test_schema");
        assert!(retrieved.is_some());
        assert_eq!(
            retrieved.unwrap().semantic_version,
            SemanticVersion::new(1, 0, 0)
        );

        Ok(())
    }

    #[test]
    fn test_migration_plan_generation() -> Result<()> {
        let mut manager = SchemaEvolutionManager::new();
        let schema_id = manager.generate_schema_id();

        let v1_schema = SchemaBuilder::new("migration_test".to_string(), FormatType::FlatBuffers)
            .with_id(schema_id)
            .version(1, 0, 0)
            .add_field(FieldDefinition::new(
                "old_field".to_string(),
                "u32".to_string(),
                true,
                1,
                0,
            ))
            .build(&mut manager)?;

        let v2_schema = SchemaBuilder::new("migration_test".to_string(), FormatType::FlatBuffers)
            .with_id(schema_id)
            .version(2, 0, 0)
            .add_field(FieldDefinition::new(
                "old_field".to_string(),
                "u64".to_string(), // Changed type - breaking
                true,
                1,
                0,
            ))
            .add_field(
                FieldDefinition::new("new_field".to_string(), "string".to_string(), true, 2, 1)
                    .with_default("default_value".to_string()),
            )
            .build(&mut manager)?;

        let migration_plan = manager.generate_migration_plan(&v1_schema, &v2_schema)?;

        assert!(!migration_plan.is_empty());
        assert!(migration_plan
            .iter()
            .any(|step| matches!(step, MigrationStep::VersionBump { is_major: true, .. })));

        Ok(())
    }
}
