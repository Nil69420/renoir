//! Schema compatibility validation for FlatBuffers and other zero-copy formats
//!
//! This module implements validation logic to ensure:
//! - Older readers can handle newer schemas (backward compatibility)
//! - Newer readers can handle older schemas (forward compatibility)
//! - Breaking changes are properly identified and require migration

use super::schema_evolution::{
    CompatibilityLevel, EvolutionAwareSchema, FieldChange, FieldDefinition,
};
use super::FormatType;
use crate::error::Result;
use std::collections::HashMap;

/// Validates schema compatibility according to zero-copy format rules
pub struct SchemaCompatibilityValidator;

impl SchemaCompatibilityValidator {
    /// Validate backward compatibility: older readers can read newer schema data
    pub fn validate_backward_compatibility(
        old_schema: &EvolutionAwareSchema,
        new_schema: &EvolutionAwareSchema,
    ) -> Result<ValidationResult> {
        if old_schema.schema_id != new_schema.schema_id {
            return Ok(ValidationResult::incompatible("Different schema types"));
        }

        let mut violations = Vec::new();
        let mut warnings = Vec::new();

        // Check field changes for backward compatibility
        for (field_name, old_field) in &old_schema.fields {
            if let Some(new_field) = new_schema.fields.get(field_name) {
                // Field exists in both schemas - check for compatible changes
                Self::validate_field_compatibility(
                    old_field,
                    new_field,
                    &mut violations,
                    &mut warnings,
                )?;
            } else {
                // Field removed from new schema
                if old_field.is_required {
                    violations.push(format!(
                        "Required field '{}' removed from schema - breaks backward compatibility",
                        field_name
                    ));
                } else {
                    warnings.push(format!(
                        "Optional field '{}' removed - may cause issues if older readers expect it",
                        field_name
                    ));
                }
            }
        }

        // Check new fields in new schema
        for (field_name, new_field) in &new_schema.fields {
            if !old_schema.fields.contains_key(field_name) {
                // New field added
                if new_field.is_required && new_field.default_value.is_none() {
                    violations.push(format!(
                        "New required field '{}' without default value breaks backward compatibility",
                        field_name
                    ));
                }
            }
        }

        // Check format-specific compatibility rules
        Self::validate_format_specific_backward_compatibility(
            old_schema,
            new_schema,
            &mut violations,
            &mut warnings,
        )?;

        Ok(ValidationResult {
            is_compatible: violations.is_empty(),
            compatibility_level: if violations.is_empty() {
                if warnings.is_empty() {
                    CompatibilityLevel::FullCompatible
                } else {
                    CompatibilityLevel::BackwardCompatible
                }
            } else {
                CompatibilityLevel::Breaking
            },
            violations,
            warnings,
        })
    }

    /// Validate forward compatibility: newer readers can read older schema data  
    pub fn validate_forward_compatibility(
        new_schema: &EvolutionAwareSchema,
        old_schema: &EvolutionAwareSchema,
    ) -> Result<ValidationResult> {
        if new_schema.schema_id != old_schema.schema_id {
            return Ok(ValidationResult::incompatible("Different schema types"));
        }

        let mut violations = Vec::new();
        let mut warnings = Vec::new();

        // Check that new reader can handle all old fields
        for (field_name, old_field) in &old_schema.fields {
            if let Some(new_field) = new_schema.fields.get(field_name) {
                // Field exists in both - check type compatibility
                if old_field.field_type != new_field.field_type {
                    if !Self::are_types_forward_compatible(
                        &old_field.field_type,
                        &new_field.field_type,
                    ) {
                        violations.push(format!(
                            "Field '{}' type changed from {} to {} - not forward compatible",
                            field_name, old_field.field_type, new_field.field_type
                        ));
                    }
                }
            } else {
                // Field missing in new schema
                if old_field.is_required {
                    violations.push(format!(
                        "Required field '{}' missing in new schema - breaks forward compatibility",
                        field_name
                    ));
                } else {
                    warnings.push(format!(
                        "Optional field '{}' missing in new schema",
                        field_name
                    ));
                }
            }
        }

        Ok(ValidationResult {
            is_compatible: violations.is_empty(),
            compatibility_level: if violations.is_empty() {
                if warnings.is_empty() {
                    CompatibilityLevel::FullCompatible
                } else {
                    CompatibilityLevel::ForwardCompatible
                }
            } else {
                CompatibilityLevel::Breaking
            },
            violations,
            warnings,
        })
    }

    /// Validate full compatibility (both directions)
    pub fn validate_full_compatibility(
        schema_a: &EvolutionAwareSchema,
        schema_b: &EvolutionAwareSchema,
    ) -> Result<ValidationResult> {
        let backward = Self::validate_backward_compatibility(schema_a, schema_b)?;
        let forward = Self::validate_forward_compatibility(schema_b, schema_a)?;

        // Combine results
        let mut violations = backward.violations;
        violations.extend(forward.violations);

        let mut warnings = backward.warnings;
        warnings.extend(forward.warnings);

        let compatibility_level = if violations.is_empty() {
            if warnings.is_empty() {
                CompatibilityLevel::FullCompatible
            } else {
                // If either direction has issues, it's not fully compatible
                match (backward.compatibility_level, forward.compatibility_level) {
                    (CompatibilityLevel::FullCompatible, CompatibilityLevel::FullCompatible) => {
                        CompatibilityLevel::FullCompatible
                    }
                    (CompatibilityLevel::BackwardCompatible, _)
                    | (_, CompatibilityLevel::BackwardCompatible) => {
                        CompatibilityLevel::BackwardCompatible
                    }
                    (CompatibilityLevel::ForwardCompatible, _)
                    | (_, CompatibilityLevel::ForwardCompatible) => {
                        CompatibilityLevel::ForwardCompatible
                    }
                    _ => CompatibilityLevel::Breaking,
                }
            }
        } else {
            CompatibilityLevel::Breaking
        };

        Ok(ValidationResult {
            is_compatible: violations.is_empty(),
            compatibility_level,
            violations,
            warnings,
        })
    }

    /// Generate field compatibility analysis
    pub fn analyze_field_changes(
        old_schema: &EvolutionAwareSchema,
        new_schema: &EvolutionAwareSchema,
    ) -> Vec<FieldChange> {
        let mut changes = Vec::new();

        // Check existing fields for modifications
        for (field_name, old_field) in &old_schema.fields {
            if let Some(new_field) = new_schema.fields.get(field_name) {
                // Field type changed
                if old_field.field_type != new_field.field_type {
                    let is_compatible = Self::are_types_backward_compatible(
                        &old_field.field_type,
                        &new_field.field_type,
                    );
                    changes.push(FieldChange::FieldTypeChanged {
                        field_name: field_name.clone(),
                        old_type: old_field.field_type.clone(),
                        new_type: new_field.field_type.clone(),
                        is_compatible,
                    });
                }

                // Required/optional status changed
                match (old_field.is_required, new_field.is_required) {
                    (true, false) => changes.push(FieldChange::FieldMadeOptional {
                        field_name: field_name.clone(),
                    }),
                    (false, true) => changes.push(FieldChange::FieldMadeRequired {
                        field_name: field_name.clone(),
                    }),
                    _ => {}
                }

                // Position changed
                if old_field.position != new_field.position {
                    changes.push(FieldChange::FieldReordered {
                        field_name: field_name.clone(),
                        old_position: old_field.position,
                        new_position: new_field.position,
                    });
                }

                // Deprecation status changed
                if !old_field.is_deprecated && new_field.is_deprecated {
                    changes.push(FieldChange::FieldDeprecated {
                        field_name: field_name.clone(),
                    });
                }
            } else {
                // Field removed
                changes.push(FieldChange::FieldRemoved {
                    field_name: field_name.clone(),
                    was_required: old_field.is_required,
                });
            }
        }

        // Check for new fields
        for (field_name, new_field) in &new_schema.fields {
            if !old_schema.fields.contains_key(field_name) {
                if new_field.is_required {
                    changes.push(FieldChange::RequiredFieldAdded {
                        field_name: field_name.clone(),
                        field_type: new_field.field_type.clone(),
                    });
                } else {
                    changes.push(FieldChange::OptionalFieldAdded {
                        field_name: field_name.clone(),
                        field_type: new_field.field_type.clone(),
                    });
                }
            }
        }

        changes
    }

    fn validate_field_compatibility(
        old_field: &FieldDefinition,
        new_field: &FieldDefinition,
        violations: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        // Type compatibility
        if old_field.field_type != new_field.field_type {
            if !Self::are_types_backward_compatible(&old_field.field_type, &new_field.field_type) {
                violations.push(format!(
                    "Field '{}' type changed from {} to {} incompatibly",
                    old_field.field_name, old_field.field_type, new_field.field_type
                ));
            } else {
                warnings.push(format!(
                    "Field '{}' type changed from {} to {} (compatible)",
                    old_field.field_name, old_field.field_type, new_field.field_type
                ));
            }
        }

        // Required/optional changes
        if old_field.is_required && !new_field.is_required {
            warnings.push(format!(
                "Field '{}' changed from required to optional",
                old_field.field_name
            ));
        } else if !old_field.is_required && new_field.is_required {
            if new_field.default_value.is_none() {
                violations.push(format!(
                    "Field '{}' changed from optional to required without default value",
                    old_field.field_name
                ));
            }
        }

        Ok(())
    }

    fn validate_format_specific_backward_compatibility(
        old_schema: &EvolutionAwareSchema,
        new_schema: &EvolutionAwareSchema,
        violations: &mut Vec<String>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        match old_schema.base.format_type {
            FormatType::FlatBuffers => Self::validate_flatbuffers_backward_compatibility(
                old_schema, new_schema, violations, warnings,
            ),
            FormatType::CapnProto => Self::validate_capnproto_backward_compatibility(
                old_schema, new_schema, violations, warnings,
            ),
            FormatType::Custom => {
                // Custom formats need custom validation
                warnings.push(
                    "Custom format compatibility cannot be automatically validated".to_string(),
                );
                Ok(())
            }
        }
    }

    fn validate_flatbuffers_backward_compatibility(
        _old_schema: &EvolutionAwareSchema,
        new_schema: &EvolutionAwareSchema,
        violations: &mut Vec<String>,
        _warnings: &mut Vec<String>,
    ) -> Result<()> {
        // FlatBuffers specific rules

        // Check field IDs are not reused with different types
        let mut field_positions: HashMap<u32, &String> = HashMap::new();
        for field in new_schema.fields.values() {
            if let Some(existing_field) = field_positions.get(&field.position) {
                if **existing_field != field.field_type {
                    violations.push(format!(
                        "Field position {} reused for different types: '{}' vs '{}'",
                        field.position, existing_field, field.field_type
                    ));
                }
            } else {
                field_positions.insert(field.position, &field.field_type);
            }
        }

        Ok(())
    }

    fn validate_capnproto_backward_compatibility(
        _old_schema: &EvolutionAwareSchema,
        _new_schema: &EvolutionAwareSchema,
        _violations: &mut Vec<String>,
        _warnings: &mut Vec<String>,
    ) -> Result<()> {
        // Cap'n Proto specific validation rules would go here
        Ok(())
    }

    fn are_types_backward_compatible(old_type: &str, new_type: &str) -> bool {
        // Define type compatibility rules
        match (old_type, new_type) {
            // Same type is always compatible
            (a, b) if a == b => true,

            // Numeric widening is backward compatible
            ("u8", "u16" | "u32" | "u64") => true,
            ("u16", "u32" | "u64") => true,
            ("u32", "u64") => true,
            ("i8", "i16" | "i32" | "i64") => true,
            ("i16", "i32" | "i64") => true,
            ("i32", "i64") => true,
            ("f32", "f64") => true,

            // String types
            ("string", "string") => true,

            // Arrays and vectors (simplified check)
            _ if old_type.starts_with("Vec<") && new_type.starts_with("Vec<") => {
                let old_inner = &old_type[4..old_type.len() - 1];
                let new_inner = &new_type[4..new_type.len() - 1];
                Self::are_types_backward_compatible(old_inner, new_inner)
            }

            _ => false,
        }
    }

    fn are_types_forward_compatible(old_type: &str, new_type: &str) -> bool {
        // Forward compatibility is more restrictive
        match (old_type, new_type) {
            // Same type is always compatible
            (a, b) if a == b => true,

            // Numeric narrowing might be forward compatible if values fit
            ("u64", "u32" | "u16" | "u8") => false, // Might overflow
            ("u32", "u16" | "u8") => false,
            ("u16", "u8") => false,
            ("i64", "i32" | "i16" | "i8") => false,
            ("i32", "i16" | "i8") => false,
            ("i16", "i8") => false,
            ("f64", "f32") => false, // Precision loss

            _ => false,
        }
    }
}

/// Result of schema compatibility validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the schemas are compatible
    pub is_compatible: bool,
    /// The level of compatibility
    pub compatibility_level: CompatibilityLevel,
    /// Compatibility violations (breaking changes)
    pub violations: Vec<String>,
    /// Warnings (non-breaking but noteworthy changes)
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub fn incompatible(reason: &str) -> Self {
        Self {
            is_compatible: false,
            compatibility_level: CompatibilityLevel::Breaking,
            violations: vec![reason.to_string()],
            warnings: vec![],
        }
    }

    pub fn compatible() -> Self {
        Self {
            is_compatible: true,
            compatibility_level: CompatibilityLevel::FullCompatible,
            violations: vec![],
            warnings: vec![],
        }
    }

    pub fn has_issues(&self) -> bool {
        !self.violations.is_empty() || !self.warnings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_formats::schema_evolution::{SchemaBuilder, SchemaEvolutionManager};

    #[test]
    fn test_backward_compatibility_optional_field_added() -> Result<()> {
        let mut manager = SchemaEvolutionManager::new();

        let old_schema = SchemaBuilder::new("test".to_string(), FormatType::FlatBuffers)
            .version(1, 0, 0)
            .add_field(FieldDefinition::new(
                "field1".to_string(),
                "u32".to_string(),
                true,
                1,
                0,
            ))
            .build(&mut manager)?;

        let new_schema = SchemaBuilder::new("test".to_string(), FormatType::FlatBuffers)
            .with_id(old_schema.schema_id)
            .version(1, 1, 0)
            .add_field(FieldDefinition::new(
                "field1".to_string(),
                "u32".to_string(),
                true,
                1,
                0,
            ))
            .add_field(FieldDefinition::new(
                "field2".to_string(),
                "string".to_string(),
                false,
                2,
                1,
            ))
            .build(&mut manager)?;

        let result = SchemaCompatibilityValidator::validate_backward_compatibility(
            &old_schema,
            &new_schema,
        )?;

        assert!(result.is_compatible);
        assert_eq!(
            result.compatibility_level,
            CompatibilityLevel::FullCompatible
        );

        Ok(())
    }

    #[test]
    fn test_backward_compatibility_required_field_added() -> Result<()> {
        let mut manager = SchemaEvolutionManager::new();

        let old_schema = SchemaBuilder::new("test".to_string(), FormatType::FlatBuffers)
            .version(1, 0, 0)
            .add_field(FieldDefinition::new(
                "field1".to_string(),
                "u32".to_string(),
                true,
                1,
                0,
            ))
            .build(&mut manager)?;

        let new_schema = SchemaBuilder::new("test".to_string(), FormatType::FlatBuffers)
            .with_id(old_schema.schema_id)
            .version(2, 0, 0)
            .add_field(FieldDefinition::new(
                "field1".to_string(),
                "u32".to_string(),
                true,
                1,
                0,
            ))
            .add_field(FieldDefinition::new(
                "field2".to_string(),
                "string".to_string(),
                true,
                2,
                1,
            ))
            .build(&mut manager)?;

        let result = SchemaCompatibilityValidator::validate_backward_compatibility(
            &old_schema,
            &new_schema,
        )?;

        assert!(!result.is_compatible);
        assert_eq!(result.compatibility_level, CompatibilityLevel::Breaking);
        assert!(!result.violations.is_empty());

        Ok(())
    }

    #[test]
    fn test_type_compatibility() {
        assert!(SchemaCompatibilityValidator::are_types_backward_compatible(
            "u32", "u64"
        ));
        assert!(!SchemaCompatibilityValidator::are_types_backward_compatible("u64", "u32"));
        assert!(SchemaCompatibilityValidator::are_types_backward_compatible(
            "f32", "f64"
        ));
        assert!(!SchemaCompatibilityValidator::are_types_forward_compatible(
            "u64", "u32"
        ));
    }
}
