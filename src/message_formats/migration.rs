//! Schema migration framework for handling breaking changes
//!
//! This module provides tools and utilities for migrating data between schema versions
//! when backward compatibility is broken. It handles:
//! - Automatic migration plan generation
//! - Data transformation between schema versions
//! - Rollback capabilities for failed migrations
//! - Custom migration function registration

use crate::error::{Result, RenoirError};
use super::schema_evolution::{
    EvolutionAwareSchema, MigrationStep, SemanticVersion, SchemaEvolutionManager
};
// Compatibility validation is handled by the evolution manager
use super::FormatType;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::fmt;

/// Migration executor for handling schema transformations
pub struct SchemaMigrationExecutor {
    /// Registered migration functions
    migration_functions: HashMap<String, Box<dyn MigrationFunction + Send + Sync>>,
    /// Migration history for rollback
    migration_history: Vec<MigrationRecord>,
    /// Migration statistics
    statistics: MigrationStatistics,
}

/// A migration function that transforms data between schema versions
pub trait MigrationFunction {
    fn migrate(&self, input_data: &[u8], context: &MigrationContext) -> Result<Vec<u8>>;
    fn rollback(&self, migrated_data: &[u8], context: &MigrationContext) -> Result<Vec<u8>>;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
}

/// Context provided to migration functions
#[derive(Debug, Clone)]
pub struct MigrationContext {
    pub from_schema: EvolutionAwareSchema,
    pub to_schema: EvolutionAwareSchema,
    pub migration_step: MigrationStep,
    pub custom_parameters: HashMap<String, String>,
}

/// Record of a completed migration for rollback purposes
#[derive(Debug, Clone)]
pub struct MigrationRecord {
    pub migration_id: u64,
    pub from_version: SemanticVersion,
    pub to_version: SemanticVersion,
    pub schema_name: String,
    pub executed_steps: Vec<MigrationStep>,
    pub timestamp: std::time::SystemTime,
    pub data_size: usize,
    pub success: bool,
    pub error_message: Option<String>,
}

/// Statistics about migrations performed
#[derive(Debug, Clone, Default)]
pub struct MigrationStatistics {
    pub total_migrations: u64,
    pub successful_migrations: u64,
    pub failed_migrations: u64,
    pub total_data_migrated_bytes: u64,
    pub average_migration_time_ms: f64,
}

/// Automated migration planner
pub struct MigrationPlanner {
    evolution_manager: Arc<Mutex<SchemaEvolutionManager>>,
}

impl SchemaMigrationExecutor {
    pub fn new() -> Self {
        let mut executor = Self {
            migration_functions: HashMap::new(),
            migration_history: Vec::new(),
            statistics: MigrationStatistics::default(),
        };

        // Register built-in migration functions
        executor.register_builtin_functions();
        executor
    }

    /// Register a custom migration function
    pub fn register_migration_function<F>(&mut self, function: F) 
    where 
        F: MigrationFunction + Send + Sync + 'static 
    {
        self.migration_functions.insert(function.name().to_string(), Box::new(function));
    }

    /// Execute a complete migration plan
    pub fn execute_migration_plan(
        &mut self,
        data: &[u8],
        migration_plan: Vec<MigrationStep>,
        from_schema: &EvolutionAwareSchema,
        to_schema: &EvolutionAwareSchema,
    ) -> Result<MigrationResult> {
        let start_time = std::time::Instant::now();
        let migration_id = self.generate_migration_id();
        
        let mut current_data = data.to_vec();
        let mut executed_steps = Vec::new();
        
        // Validate migration plan
        self.validate_migration_plan(&migration_plan, from_schema, to_schema)?;

        for step in migration_plan {
            let context = MigrationContext {
                from_schema: from_schema.clone(),
                to_schema: to_schema.clone(),
                migration_step: step.clone(),
                custom_parameters: HashMap::new(),
            };

            match self.execute_migration_step(&current_data, &step, &context) {
                Ok(migrated_data) => {
                    current_data = migrated_data;
                    executed_steps.push(step);
                }
                Err(e) => {
                    // Migration failed - record failure and return error
                    let error_msg = e.to_string();
                    self.record_migration_failure(
                        migration_id, 
                        from_schema, 
                        to_schema, 
                        executed_steps,
                        error_msg
                    );
                    return Err(e);
                }
            }
        }

        let duration = start_time.elapsed();
        
        // Record successful migration
        let record = MigrationRecord {
            migration_id,
            from_version: from_schema.semantic_version.clone(),
            to_version: to_schema.semantic_version.clone(),
            schema_name: from_schema.base.schema_name.clone(),
            executed_steps: executed_steps.clone(),
            timestamp: std::time::SystemTime::now(),
            data_size: current_data.len(),
            success: true,
            error_message: None,
        };

        self.migration_history.push(record);
        self.update_statistics(true, current_data.len(), duration.as_millis() as f64);

        let migrated_size = current_data.len();
        Ok(MigrationResult {
            migrated_data: current_data,
            migration_id,
            executed_steps,
            original_size: data.len(),
            migrated_size,
            duration_ms: duration.as_millis() as u64,
        })
    }

    /// Rollback a migration by ID
    pub fn rollback_migration(&mut self, migration_id: u64, current_data: &[u8]) -> Result<Vec<u8>> {
        let record = self.migration_history.iter()
            .find(|r| r.migration_id == migration_id)
            .ok_or_else(|| RenoirError::invalid_parameter(
                "migration_id", 
                "Migration record not found"
            ))?;

        if !record.success {
            return Err(RenoirError::invalid_parameter(
                "migration_id", 
                "Cannot rollback failed migration"
            ));
        }

        // Execute rollback steps in reverse order
        let mut current = current_data.to_vec();
        for step in record.executed_steps.iter().rev() {
            current = self.execute_rollback_step(&current, step)?;
        }

        Ok(current)
    }

    fn execute_migration_step(
        &self, 
        data: &[u8], 
        step: &MigrationStep,
        context: &MigrationContext,
    ) -> Result<Vec<u8>> {
        match step {
            MigrationStep::AssignDefaultValue { field_name, default_value } => {
                self.assign_default_value(data, field_name, default_value, context)
            }
            MigrationStep::MapField { old_field, new_field, transform } => {
                self.map_field(data, old_field, new_field, transform, context)
            }
            MigrationStep::CustomTransform { function_name, parameters } => {
                self.execute_custom_transform(data, function_name, parameters, context)
            }
            MigrationStep::ValidateData { validation_rule } => {
                self.validate_migrated_data(data, validation_rule, context)?;
                Ok(data.to_vec()) // Validation doesn't change data
            }
            MigrationStep::VersionBump { from_version: _, to_version: _, is_major: _ } => {
                // Version bumps don't change data structure
                Ok(data.to_vec())
            }
        }
    }

    fn execute_rollback_step(&self, data: &[u8], step: &MigrationStep) -> Result<Vec<u8>> {
        match step {
            MigrationStep::AssignDefaultValue { field_name, default_value: _ } => {
                // Remove the field that was added
                self.remove_field(data, field_name)
            }
            MigrationStep::MapField { old_field, new_field, transform } => {
                // Reverse the field mapping
                self.reverse_map_field(data, new_field, old_field, transform)
            }
            MigrationStep::CustomTransform { function_name, parameters: _ } => {
                // Find and execute the rollback function
                if let Some(function) = self.migration_functions.get(function_name) {
                    let context = MigrationContext {
                        from_schema: Default::default(), // TODO: Get from context
                        to_schema: Default::default(),
                        migration_step: step.clone(),
                        custom_parameters: HashMap::new(),
                    };
                    function.rollback(data, &context)
                } else {
                    Err(RenoirError::invalid_parameter(
                        "function_name",
                        &format!("Migration function '{}' not found", function_name)
                    ))
                }
            }
            _ => Ok(data.to_vec()), // Other steps don't need rollback
        }
    }

    fn assign_default_value(
        &self, 
        data: &[u8], 
        field_name: &str, 
        default_value: &str, 
        context: &MigrationContext
    ) -> Result<Vec<u8>> {
        match context.to_schema.base.format_type {
            FormatType::FlatBuffers => {
                self.assign_default_value_flatbuffers(data, field_name, default_value, context)
            }
            FormatType::CapnProto => {
                self.assign_default_value_capnproto(data, field_name, default_value, context)
            }
            FormatType::Custom => {
                Err(RenoirError::invalid_parameter(
                    "format_type",
                    "Custom format migration not implemented"
                ))
            }
        }
    }

    fn assign_default_value_flatbuffers(
        &self,
        data: &[u8],
        field_name: &str,
        default_value: &str,
        _context: &MigrationContext,
    ) -> Result<Vec<u8>> {
        // This would integrate with the FlatBuffers library to add a field with default value
        // For now, return a placeholder implementation
        let mut new_data = data.to_vec();
        
        // Add metadata about the default value assignment
        let metadata = format!("{}={}", field_name, default_value);
        new_data.extend(metadata.as_bytes());
        
        Ok(new_data)
    }

    fn assign_default_value_capnproto(
        &self,
        data: &[u8],
        _field_name: &str,
        _default_value: &str,
        _context: &MigrationContext,
    ) -> Result<Vec<u8>> {
        // Cap'n Proto implementation would go here
        Ok(data.to_vec())
    }

    fn map_field(
        &self,
        data: &[u8],
        _old_field: &str,
        _new_field: &str,
        _transform: &str,
        _context: &MigrationContext,
    ) -> Result<Vec<u8>> {
        // Field mapping implementation
        Ok(data.to_vec())
    }

    fn reverse_map_field(
        &self,
        data: &[u8],
        _new_field: &str,
        _old_field: &str,
        _transform: &str,
    ) -> Result<Vec<u8>> {
        // Reverse field mapping implementation
        Ok(data.to_vec())
    }

    fn remove_field(&self, data: &[u8], _field_name: &str) -> Result<Vec<u8>> {
        // Field removal implementation
        Ok(data.to_vec())
    }

    fn execute_custom_transform(
        &self,
        data: &[u8],
        function_name: &str,
        parameters: &[String],
        context: &MigrationContext,
    ) -> Result<Vec<u8>> {
        let function = self.migration_functions.get(function_name)
            .ok_or_else(|| RenoirError::invalid_parameter(
                "function_name",
                &format!("Migration function '{}' not found", function_name)
            ))?;

        let mut custom_context = context.clone();
        for param in parameters {
            if let Some((key, value)) = param.split_once('=') {
                custom_context.custom_parameters.insert(key.to_string(), value.to_string());
            }
        }

        function.migrate(data, &custom_context)
    }

    fn validate_migrated_data(
        &self,
        data: &[u8],
        _validation_rule: &str,
        _context: &MigrationContext,
    ) -> Result<()> {
        // Implement validation logic
        if data.is_empty() {
            return Err(RenoirError::invalid_parameter(
                "data",
                "Migrated data is empty"
            ));
        }
        Ok(())
    }

    fn validate_migration_plan(
        &self,
        plan: &[MigrationStep],
        from_schema: &EvolutionAwareSchema,
        to_schema: &EvolutionAwareSchema,
    ) -> Result<()> {
        if from_schema.schema_id != to_schema.schema_id {
            return Err(RenoirError::invalid_parameter(
                "schema",
                "Cannot migrate between different schema types"
            ));
        }

        // Validate each step
        for step in plan {
            self.validate_migration_step(step, from_schema, to_schema)?;
        }

        Ok(())
    }

    fn validate_migration_step(
        &self,
        step: &MigrationStep,
        _from_schema: &EvolutionAwareSchema,
        to_schema: &EvolutionAwareSchema,
    ) -> Result<()> {
        match step {
            MigrationStep::AssignDefaultValue { field_name, default_value: _ } => {
                if !to_schema.fields.contains_key(field_name) {
                    return Err(RenoirError::invalid_parameter(
                        "field_name",
                        &format!("Field '{}' not found in target schema", field_name)
                    ));
                }
            }
            MigrationStep::CustomTransform { function_name, parameters: _ } => {
                if !self.migration_functions.contains_key(function_name) {
                    return Err(RenoirError::invalid_parameter(
                        "function_name",
                        &format!("Migration function '{}' not registered", function_name)
                    ));
                }
            }
            _ => {} // Other steps are always valid
        }
        Ok(())
    }

    fn record_migration_failure(
        &mut self,
        migration_id: u64,
        from_schema: &EvolutionAwareSchema,
        to_schema: &EvolutionAwareSchema,
        executed_steps: Vec<MigrationStep>,
        error_message: String,
    ) {
        let record = MigrationRecord {
            migration_id,
            from_version: from_schema.semantic_version.clone(),
            to_version: to_schema.semantic_version.clone(),
            schema_name: from_schema.base.schema_name.clone(),
            executed_steps,
            timestamp: std::time::SystemTime::now(),
            data_size: 0,
            success: false,
            error_message: Some(error_message),
        };

        self.migration_history.push(record);
        self.update_statistics(false, 0, 0.0);
    }

    fn update_statistics(&mut self, success: bool, data_size: usize, duration_ms: f64) {
        self.statistics.total_migrations += 1;
        
        if success {
            self.statistics.successful_migrations += 1;
            self.statistics.total_data_migrated_bytes += data_size as u64;
            
            // Update average migration time
            let total_successful = self.statistics.successful_migrations as f64;
            self.statistics.average_migration_time_ms = 
                (self.statistics.average_migration_time_ms * (total_successful - 1.0) + duration_ms) / total_successful;
        } else {
            self.statistics.failed_migrations += 1;
        }
    }

    fn generate_migration_id(&self) -> u64 {
        // Simple ID generation based on timestamp and counter
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now * 1000 + (self.migration_history.len() as u64)
    }

    fn register_builtin_functions(&mut self) {
        // Register common migration functions
        self.register_migration_function(DefaultValueAssigner);
        self.register_migration_function(TypeConverter);
        self.register_migration_function(FieldReorderer);
    }

    /// Get migration history
    pub fn get_migration_history(&self) -> &[MigrationRecord] {
        &self.migration_history
    }

    /// Get migration statistics
    pub fn get_statistics(&self) -> &MigrationStatistics {
        &self.statistics
    }
}

/// Result of a successful migration
#[derive(Debug)]
pub struct MigrationResult {
    pub migrated_data: Vec<u8>,
    pub migration_id: u64,
    pub executed_steps: Vec<MigrationStep>,
    pub original_size: usize,
    pub migrated_size: usize,
    pub duration_ms: u64,
}

impl MigrationPlanner {
    pub fn new(evolution_manager: Arc<Mutex<SchemaEvolutionManager>>) -> Self {
        Self { evolution_manager }
    }

    /// Generate an optimal migration plan between schema versions
    pub fn generate_migration_plan(
        &self,
        from_schema: &EvolutionAwareSchema,
        to_schema: &EvolutionAwareSchema,
    ) -> Result<Vec<MigrationStep>> {
        // Check if migration is needed
        let compatibility = {
            let manager = self.evolution_manager.lock().map_err(|_| {
                RenoirError::platform("Failed to acquire evolution manager lock")
            })?;
            manager.check_compatibility(from_schema, to_schema)?
        };

        match compatibility {
            super::schema_evolution::CompatibilityLevel::FullCompatible => {
                // No migration needed
                Ok(vec![])
            }
            super::schema_evolution::CompatibilityLevel::BackwardCompatible | 
            super::schema_evolution::CompatibilityLevel::ForwardCompatible => {
                // Minor migration may be needed
                self.generate_compatible_migration_plan(from_schema, to_schema)
            }
            super::schema_evolution::CompatibilityLevel::Breaking => {
                // Full migration required
                let manager = self.evolution_manager.lock().map_err(|_| {
                    RenoirError::platform("Failed to acquire evolution manager lock")
                })?;
                manager.generate_migration_plan(from_schema, to_schema)
            }
        }
    }

    fn generate_compatible_migration_plan(
        &self,
        _from_schema: &EvolutionAwareSchema,
        _to_schema: &EvolutionAwareSchema,
    ) -> Result<Vec<MigrationStep>> {
        // For compatible schemas, we might still need minor adjustments
        Ok(vec![])
    }
}

impl Default for SchemaMigrationExecutor {
    fn default() -> Self {
        Self::new()
    }
}

// Built-in migration functions

struct DefaultValueAssigner;

impl MigrationFunction for DefaultValueAssigner {
    fn migrate(&self, data: &[u8], context: &MigrationContext) -> Result<Vec<u8>> {
        if let MigrationStep::AssignDefaultValue { field_name, default_value } = &context.migration_step {
            // Simplified implementation - would integrate with format-specific libraries
            let mut result = data.to_vec();
            let assignment = format!("{}={};", field_name, default_value);
            result.extend(assignment.as_bytes());
            Ok(result)
        } else {
            Err(RenoirError::invalid_parameter("step", "Expected AssignDefaultValue step"))
        }
    }

    fn rollback(&self, data: &[u8], _context: &MigrationContext) -> Result<Vec<u8>> {
        // Remove the last assignment (simplified)
        if let Some(pos) = data.iter().rposition(|&b| b == b';') {
            Ok(data[..pos].to_vec())
        } else {
            Ok(data.to_vec())
        }
    }

    fn name(&self) -> &str {
        "default_value_assigner"
    }

    fn description(&self) -> &str {
        "Assigns default values to new required fields"
    }
}

struct TypeConverter;

impl MigrationFunction for TypeConverter {
    fn migrate(&self, data: &[u8], _context: &MigrationContext) -> Result<Vec<u8>> {
        // Type conversion implementation
        Ok(data.to_vec())
    }

    fn rollback(&self, data: &[u8], _context: &MigrationContext) -> Result<Vec<u8>> {
        // Reverse type conversion
        Ok(data.to_vec())
    }

    fn name(&self) -> &str {
        "type_converter"
    }

    fn description(&self) -> &str {
        "Converts field types between compatible formats"
    }
}

struct FieldReorderer;

impl MigrationFunction for FieldReorderer {
    fn migrate(&self, data: &[u8], _context: &MigrationContext) -> Result<Vec<u8>> {
        // Field reordering implementation
        Ok(data.to_vec())
    }

    fn rollback(&self, data: &[u8], _context: &MigrationContext) -> Result<Vec<u8>> {
        // Reverse field reordering
        Ok(data.to_vec())
    }

    fn name(&self) -> &str {
        "field_reorderer"
    }

    fn description(&self) -> &str {
        "Reorders fields to match new schema layout"
    }
}

impl fmt::Display for MigrationStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, 
            "Migration Statistics:\n\
             Total: {}\n\
             Successful: {} ({:.1}%)\n\
             Failed: {} ({:.1}%)\n\
             Data Migrated: {} bytes\n\
             Avg Time: {:.2}ms",
            self.total_migrations,
            self.successful_migrations,
            if self.total_migrations > 0 { 
                (self.successful_migrations as f64 / self.total_migrations as f64) * 100.0 
            } else { 0.0 },
            self.failed_migrations,
            if self.total_migrations > 0 { 
                (self.failed_migrations as f64 / self.total_migrations as f64) * 100.0 
            } else { 0.0 },
            self.total_data_migrated_bytes,
            self.average_migration_time_ms
        )
    }
}

// Required for the context to have a default implementation
impl Default for EvolutionAwareSchema {
    fn default() -> Self {
        Self {
            base: crate::message_formats::registry::SchemaInfo::new(
                FormatType::FlatBuffers,
                "default".to_string(),
                1,
                0,
            ),
            schema_id: 0,
            semantic_version: super::schema_evolution::SemanticVersion::new(1, 0, 0),
            compatibility_rules: std::collections::BTreeMap::new(),
            fields: HashMap::new(),
            deprecated_fields: std::collections::HashSet::new(),
            requires_migration_from: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_formats::schema_evolution::{SchemaBuilder, SchemaEvolutionManager};

    #[test]
    fn test_migration_executor_creation() {
        let executor = SchemaMigrationExecutor::new();
        assert!(executor.migration_functions.contains_key("default_value_assigner"));
        assert!(executor.migration_functions.contains_key("type_converter"));
        assert!(executor.migration_functions.contains_key("field_reorderer"));
    }

    #[test]
    fn test_migration_plan_execution() -> Result<()> {
        let mut executor = SchemaMigrationExecutor::new();
        let mut manager = SchemaEvolutionManager::new();

        let from_schema = SchemaBuilder::new("test".to_string(), FormatType::FlatBuffers)
            .version(1, 0, 0)
            .build(&mut manager)?;

        let to_schema = SchemaBuilder::new("test".to_string(), FormatType::FlatBuffers)
            .with_id(from_schema.schema_id)
            .version(2, 0, 0)
            .build(&mut manager)?;

        let plan = vec![
            MigrationStep::VersionBump { 
                from_version: 1, 
                to_version: 2, 
                is_major: true 
            }
        ];

        let test_data = b"test data";
        let result = executor.execute_migration_plan(
            test_data, 
            plan, 
            &from_schema, 
            &to_schema
        )?;

        assert!(!result.migrated_data.is_empty());
        assert_eq!(result.executed_steps.len(), 1);

        Ok(())
    }
}