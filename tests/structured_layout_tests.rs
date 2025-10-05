//! Tests for structured memory layout

#[cfg(test)]
mod tests {
    use renoir::{
        memory::{BackingType, RegionConfig, SharedMemoryManager},
        structured_layout::{StructuredMemoryRegion, RENOIR_MAGIC, SCHEMA_VERSION},
    };

    #[test]
    fn test_structured_layout_basic() {
        let manager = SharedMemoryManager::new();
        let config = RegionConfig {
            name: "test_structured".to_string(),
            size: 1024 * 1024,
            backing_type: BackingType::MemFd,
            file_path: None,
            create: true,
            permissions: 0o600,
        };
        let region = manager.create_region(config).unwrap();

        let mut structured = StructuredMemoryRegion::create(region, true).unwrap();

        // Test global header validation
        let stats = structured.get_global_stats();
        assert_eq!(stats.magic, RENOIR_MAGIC);
        assert_eq!(stats.version, SCHEMA_VERSION);
        assert_eq!(stats.topic_count, 0);

        // Test topic creation
        let topic_id = structured.create_topic("test_topic", 1024, 1).unwrap();
        assert!(topic_id > 0);

        // Test topic lookup
        let entry = structured.find_topic("test_topic").unwrap();
        assert_eq!(entry.topic_id, topic_id);
        assert_eq!(entry.ring_size, 1024);
        assert_eq!(entry.schema_id, 1);

        let stats = structured.get_global_stats();
        assert_eq!(stats.topic_count, 1);
    }
}
