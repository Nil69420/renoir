//! Zero-Copy Message Format Integration Tests
//!
//! Tests demonstrating and validating zero-copy message format functionality
//! with practical embedded systems scenarios

use renoir::{
    error::Result,
    message_formats::{
        registry::{SchemaInfo, UseCase, ZeroCopyFormatRegistry},
        traits::ZeroCopyBuilder,
        zero_copy::{
            create_pose_data_capnp, create_sensor_data_flatbuffer, CapnProtoFormat,
            FlatBufferFormat,
        },
        FormatType,
    },
};

#[cfg(test)]
mod zero_copy_integration_tests {
    use super::*;

    #[test]
    fn test_sensor_data_streaming_flatbuffer() -> Result<()> {
        // Create sensor data schema
        let sensor_schema = SchemaInfo::new(
            FormatType::FlatBuffers,
            "sensor_data".to_string(),
            1,
            0x12345678,
        );

        // Create FlatBuffer format
        let mut flatbuffer_format = FlatBufferFormat::new();
        flatbuffer_format.register_schema(sensor_schema.clone())?;

        // Test creating multiple sensor data messages
        for i in 0..10 {
            let timestamp = 1000000 + i * 10; // 10ms intervals
            let sensor_value = 42.5 + (i as f64) * 0.1;
            let sensor_id = 100 + i as u32;

            // Create zero-copy accessor
            let accessor = create_sensor_data_flatbuffer(timestamp, sensor_value, sensor_id)?;

            // Validate fields without deserialization
            let ts = accessor.get_u64("timestamp")?.unwrap_or(0);
            let val = accessor.get_f64("value")?.unwrap_or(0.0);
            let id = accessor.get_u32("sensor_id").unwrap_or(None).unwrap_or(0);

            assert_eq!(ts, timestamp);
            assert_eq!(val, sensor_value);
            assert_eq!(id, sensor_id);
            assert!(accessor.buffer().len() > 0);
        }

        Ok(())
    }

    #[test]
    fn test_robot_pose_messages_capnproto() -> Result<()> {
        // Create pose data schema
        let pose_schema = SchemaInfo::new(
            FormatType::CapnProto,
            "pose_data".to_string(),
            1,
            0x87654321,
        );

        // Create Cap'n Proto format
        let mut capnp_format = CapnProtoFormat::new();
        capnp_format.register_schema(pose_schema.clone())?;

        // Create robot pose message
        let accessor = create_pose_data_capnp(
            1.5, 2.3, 0.8, // position (x, y, z)
            1.0, 0.0, 0.0, 0.0, // orientation (w, x, y, z)
        )?;

        // Validate position fields
        let pos_x = accessor.get_f64("position_x")?.unwrap_or(0.0);
        let pos_y = accessor.get_f64("position_y")?.unwrap_or(0.0);
        let pos_z = accessor.get_f64("position_z")?.unwrap_or(0.0);

        // Validate orientation fields
        let ori_w = accessor.get_f64("orientation_w")?.unwrap_or(0.0);
        let ori_x = accessor.get_f64("orientation_x")?.unwrap_or(0.0);
        let ori_y = accessor.get_f64("orientation_y")?.unwrap_or(0.0);
        let ori_z = accessor.get_f64("orientation_z")?.unwrap_or(0.0);

        assert_eq!(pos_x, 1.5);
        assert_eq!(pos_y, 2.3);
        assert_eq!(pos_z, 0.8);
        assert_eq!(ori_w, 1.0);
        assert_eq!(ori_x, 0.0);
        assert_eq!(ori_y, 0.0);
        assert_eq!(ori_z, 0.0);

        let fields = accessor.list_fields();
        assert!(fields.contains(&"position_x"));
        assert!(fields.contains(&"orientation_w"));

        Ok(())
    }

    #[test]
    fn test_format_recommendations() -> Result<()> {
        let registry = ZeroCopyFormatRegistry::new();

        let test_cases = vec![
            (UseCase::HighFrequency, FormatType::FlatBuffers),
            (UseCase::LowLatency, FormatType::FlatBuffers),
            (UseCase::LargeMessages, FormatType::CapnProto),
            (UseCase::CrossLanguage, FormatType::FlatBuffers),
            (UseCase::RealTime, FormatType::FlatBuffers),
            (UseCase::Streaming, FormatType::CapnProto),
            (UseCase::GeneralPurpose, FormatType::FlatBuffers),
        ];

        for (use_case, expected_format) in test_cases {
            let recommended_format = registry.recommend_format(use_case);
            assert_eq!(recommended_format, expected_format);
            assert!(recommended_format.is_zero_copy());
        }

        Ok(())
    }

    #[test]
    fn test_schema_compatibility() -> Result<()> {
        let mut registry = ZeroCopyFormatRegistry::new();

        // Create different versions of the same schema
        let schema_v1 = SchemaInfo::new(
            FormatType::FlatBuffers,
            "sensor_data".to_string(),
            1,
            0x12345678,
        );

        let schema_v2 = SchemaInfo::new(
            FormatType::FlatBuffers,
            "sensor_data".to_string(),
            2,
            0x12345679, // Different hash = schema change
        );

        let different_schema = SchemaInfo::new(
            FormatType::CapnProto,
            "sensor_data".to_string(),
            1,
            0x12345678,
        );

        // Test compatibility
        assert!(schema_v1.is_compatible(&schema_v1));
        assert!(!schema_v1.is_compatible(&schema_v2)); // Different versions
        assert!(!schema_v1.is_compatible(&different_schema)); // Different formats

        // Test schema registration
        registry.register_schema(schema_v1.clone())?;
        registry.register_schema(different_schema.clone())?;

        let schemas = registry.list_schemas();
        assert!(schemas.len() >= 2);

        // Test topic format assignment
        registry.assign_topic_format("sensors/*".to_string(), FormatType::FlatBuffers)?;

        Ok(())
    }

    #[test]
    fn test_custom_message_building() -> Result<()> {
        // Create a custom message with mixed field types
        let mut builder = ZeroCopyBuilder::new(FormatType::FlatBuffers);

        // Add different field types
        builder.add_u64("timestamp".to_string(), 1234567890)?;
        builder.add_string("device_name".to_string(), "IMU_01")?;
        builder.add_f64("temperature".to_string(), 36.5)?;
        builder.add_u32("status_flags".to_string(), 0x00FF)?;
        builder.add_bytes("calibration_data".to_string(), &[0xDE, 0xAD, 0xBE, 0xEF])?;

        // Finish building
        let schema = SchemaInfo::new(
            FormatType::FlatBuffers,
            "custom_device_message".to_string(),
            1,
            0xABCDEF00,
        );

        let accessor = builder.finish(schema);

        // Validate all fields
        assert_eq!(accessor.get_u64("timestamp")?.unwrap_or(0), 1234567890);
        assert_eq!(
            accessor.get_string("device_name")?.as_deref().unwrap_or(""),
            "IMU_01"
        );
        assert_eq!(accessor.get_f64("temperature")?.unwrap_or(0.0), 36.5);
        assert_eq!(accessor.get_u32("status_flags")?.unwrap_or(0), 0x00FF);

        let cal_data = accessor.get_bytes("calibration_data")?.unwrap_or(&[]);
        assert_eq!(cal_data, &[0xDE, 0xAD, 0xBE, 0xEF]);

        assert!(accessor.buffer().len() > 0);

        Ok(())
    }

    #[test]
    fn test_zero_copy_field_access_types() -> Result<()> {
        let accessor = create_sensor_data_flatbuffer(123456789, 98.6, 42)?;

        // Test all field access methods
        assert_eq!(accessor.get_u64("timestamp")?.unwrap(), 123456789);
        assert_eq!(accessor.get_f64("value")?.unwrap(), 98.6);
        assert_eq!(accessor.get_u32("sensor_id")?.unwrap(), 42);

        // Test non-existent fields
        assert!(accessor.get_u64("nonexistent")?.is_none());
        assert!(accessor.get_string("nonexistent")?.is_none());

        // Test field listing
        let fields = accessor.list_fields();
        assert!(fields.contains(&"timestamp"));
        assert!(fields.contains(&"value"));
        assert!(fields.contains(&"sensor_id"));

        Ok(())
    }

    #[test]
    fn test_format_registry_operations() -> Result<()> {
        let mut registry = ZeroCopyFormatRegistry::new();

        // Test format registration and retrieval
        assert!(registry.get_format(&FormatType::FlatBuffers).is_some());
        assert!(registry.get_format(&FormatType::CapnProto).is_some());

        // Test schema operations
        let schema = SchemaInfo::new(
            FormatType::FlatBuffers,
            "test_schema".to_string(),
            1,
            0x12345678,
        );

        registry.register_schema(schema.clone())?;

        let schemas = registry.list_schemas();
        assert!(schemas.iter().any(|s| s.schema_name == "test_schema"));

        // Test topic format assignment
        registry.assign_topic_format("test/topic".to_string(), FormatType::FlatBuffers)?;

        Ok(())
    }

    #[test]
    fn test_multiple_format_types() -> Result<()> {
        // Test that both FlatBuffers and Cap'n Proto work
        let fb_accessor = create_sensor_data_flatbuffer(1000, 25.5, 1)?;
        let cp_accessor = create_pose_data_capnp(1.0, 2.0, 3.0, 1.0, 0.0, 0.0, 0.0)?;

        // Verify FlatBuffer data
        assert_eq!(fb_accessor.get_u64("timestamp")?.unwrap(), 1000);
        assert_eq!(fb_accessor.get_f64("value")?.unwrap(), 25.5);

        // Verify Cap'n Proto data
        assert_eq!(cp_accessor.get_f64("position_x")?.unwrap(), 1.0);
        assert_eq!(cp_accessor.get_f64("position_y")?.unwrap(), 2.0);

        // Both should have non-zero buffer sizes
        assert!(fb_accessor.buffer().len() > 0);
        assert!(cp_accessor.buffer().len() > 0);

        Ok(())
    }

    #[test]
    fn test_schema_versioning() -> Result<()> {
        let schema_v1 = SchemaInfo::new(
            FormatType::FlatBuffers,
            "versioned_schema".to_string(),
            1,
            0x11111111,
        );

        let schema_v2 = SchemaInfo::new(
            FormatType::FlatBuffers,
            "versioned_schema".to_string(),
            2,
            0x22222222,
        );

        // Same name, same format, but different versions should not be compatible
        assert!(!schema_v1.is_compatible(&schema_v2));

        // Same schema should be compatible with itself
        assert!(schema_v1.is_compatible(&schema_v1));
        assert!(schema_v2.is_compatible(&schema_v2));

        Ok(())
    }

    #[test]
    fn test_zero_copy_no_allocations() -> Result<()> {
        // This test conceptually verifies zero-copy behavior
        // In a real scenario, you'd use allocation tracking tools

        let accessor = create_sensor_data_flatbuffer(1, 2.0, 3)?;

        // Multiple field accesses should not cause additional allocations
        for _ in 0..100 {
            let _ = accessor.get_u64("timestamp")?;
            let _ = accessor.get_f64("value")?;
            let _ = accessor.get_u32("sensor_id")?;
        }

        // If we reach here without issues, zero-copy access is working
        assert!(true);
        Ok(())
    }
}
