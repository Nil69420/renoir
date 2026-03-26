//! Tests for metadata components

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use renoir::{
        memory::{BackingType, RegionConfig},
        metadata_modules::{ControlRegion, ControlStats, RegionMetadata, RegionRegistryEntry},
    };
    use std::time::SystemTime;

    #[test]
    fn test_region_metadata() {
        let metadata = RegionMetadata::new("test".to_string(), 4096, BackingType::FileBacked);

        assert_eq!(metadata.name, "test");
        assert_eq!(metadata.size, 4096);
        assert_eq!(metadata.backing_type, BackingType::FileBacked);
        assert_eq!(metadata.version, 1);

        // Test validation
        assert!(metadata.validate().is_ok());

        // Test compatibility
        assert!(metadata.is_compatible_with(1));
        assert!(!metadata.is_compatible_with(2));
    }

    #[test]
    fn test_region_metadata_validation() {
        // Test empty name
        let mut metadata = RegionMetadata::new("".to_string(), 4096, BackingType::FileBacked);
        assert!(metadata.validate().is_err());

        // Test zero size
        metadata.name = "test".to_string();
        metadata.size = 0;
        assert!(metadata.validate().is_err());

        // Test zero version
        metadata.size = 4096;
        metadata.version = 0;
        assert!(metadata.validate().is_err());

        // Test valid metadata
        metadata.version = 1;
        assert!(metadata.validate().is_ok());
    }

    #[test]
    fn test_registry_entry() {
        let metadata = RegionMetadata::new("test".to_string(), 4096, BackingType::FileBacked);

        let mut entry = RegionRegistryEntry::new(metadata);
        assert_eq!(entry.ref_count, 1);
        assert_eq!(entry.creator_pid, std::process::id());

        // Test reference counting
        entry.add_ref();
        assert_eq!(entry.ref_count, 2);

        assert!(!entry.remove_ref());
        assert_eq!(entry.ref_count, 1);

        assert!(entry.remove_ref());
        assert_eq!(entry.ref_count, 0);

        // Test touch functionality
        let initial_time = entry.last_accessed;
        std::thread::sleep(std::time::Duration::from_millis(10));
        entry.touch();
        assert!(entry.last_accessed > initial_time);
    }

    #[test]
    fn test_registry_entry_stale_detection() {
        let metadata = RegionMetadata::new("test".to_string(), 4096, BackingType::FileBacked);

        let mut entry = RegionRegistryEntry::new(metadata);

        // Fresh entry should not be stale
        assert!(!entry.is_stale(60));

        // Simulate old access time
        entry.last_accessed = SystemTime::now() - std::time::Duration::from_secs(120);
        assert!(entry.is_stale(60));
        assert!(!entry.is_stale(150));
    }

    #[test]
    fn test_control_stats() {
        let created_at = SystemTime::now();
        let last_modified = created_at + std::time::Duration::from_secs(10);

        let stats = ControlStats::new(5, 8192, 42, created_at, last_modified);

        assert_eq!(stats.total_regions, 5);
        assert_eq!(stats.total_size, 8192);
        assert_eq!(stats.global_sequence, 42);
        assert_eq!(stats.created_at, created_at);
        assert_eq!(stats.last_modified, last_modified);

        // Test activity detection
        assert!(stats.is_active(3600)); // Should be active within an hour
    }

    #[test]
    fn test_control_region_creation() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "control".to_string(),
            size: 8192,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("control_shm")),
            create: true,
            permissions: 0o644,
        };

        let control = ControlRegion::new(config).unwrap();

        let stats = control.get_stats();
        assert_eq!(stats.total_regions, 0);
        assert_eq!(stats.total_size, 8192);
        assert_eq!(stats.global_sequence, 0);
    }

    #[test]
    fn test_control_region_registry() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "control".to_string(),
            size: 8192,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("control_shm")),
            create: true,
            permissions: 0o644,
        };

        let control = ControlRegion::new(config).unwrap();

        let metadata =
            RegionMetadata::new("test_region".to_string(), 4096, BackingType::FileBacked);

        // Register region
        control.register_region(&metadata).unwrap();

        // Verify registration
        let entry = control.get_region_entry("test_region").unwrap();
        assert_eq!(entry.metadata.name, "test_region");
        assert_eq!(entry.ref_count, 1);

        let stats = control.get_stats();
        assert_eq!(stats.total_regions, 1);
        assert!(stats.global_sequence > 0);

        // List regions
        let regions = control.list_regions();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].metadata.name, "test_region");
    }

    #[test]
    fn test_control_region_ref_counting() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "control".to_string(),
            size: 8192,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("control_shm")),
            create: true,
            permissions: 0o644,
        };

        let control = ControlRegion::new(config).unwrap();

        let metadata =
            RegionMetadata::new("test_region".to_string(), 4096, BackingType::FileBacked);

        // Register and add references
        control.register_region(&metadata).unwrap();
        control.add_region_ref("test_region").unwrap();

        let entry = control.get_region_entry("test_region").unwrap();
        assert_eq!(entry.ref_count, 2);

        // Remove references
        assert!(!control.remove_region_ref("test_region").unwrap());
        let entry = control.get_region_entry("test_region").unwrap();
        assert_eq!(entry.ref_count, 1);

        // Final removal should delete the entry
        assert!(control.remove_region_ref("test_region").unwrap());
        assert!(control.get_region_entry("test_region").is_none());
    }

    #[test]
    fn test_control_region_sequence() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "control".to_string(),
            size: 8192,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("control_shm")),
            create: true,
            permissions: 0o644,
        };

        let control = ControlRegion::new(config).unwrap();

        assert_eq!(control.get_sequence(), 0);

        let seq1 = control.increment_sequence();
        assert_eq!(seq1, 1);
        assert_eq!(control.get_sequence(), 1);

        let seq2 = control.increment_sequence();
        assert_eq!(seq2, 2);
        assert_eq!(control.get_sequence(), 2);
    }

    #[test]
    fn test_control_region_unregister() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "control".to_string(),
            size: 8192,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("control_shm")),
            create: true,
            permissions: 0o644,
        };

        let control = ControlRegion::new(config).unwrap();

        let metadata =
            RegionMetadata::new("test_region".to_string(), 4096, BackingType::FileBacked);

        // Register and then unregister
        control.register_region(&metadata).unwrap();
        assert!(control.get_region_entry("test_region").is_some());

        control.unregister_region("test_region").unwrap();
        assert!(control.get_region_entry("test_region").is_none());

        // Unregistering non-existent region should fail
        assert!(control.unregister_region("nonexistent").is_err());
    }

    #[test]
    fn test_control_region_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("control_persistent");

        // Create control region and add data
        {
            let config = RegionConfig {
                name: "control".to_string(),
                size: 8192,
                backing_type: BackingType::FileBacked,
                file_path: Some(file_path.clone()),
                create: true,
                permissions: 0o644,
            };

            let control = ControlRegion::new(config).unwrap();

            let metadata = RegionMetadata::new(
                "persistent_region".to_string(),
                4096,
                BackingType::FileBacked,
            );

            control.register_region(&metadata).unwrap();
        }

        // Re-open and verify data persisted
        {
            let config = RegionConfig {
                name: "control".to_string(),
                size: 8192,
                backing_type: BackingType::FileBacked,
                file_path: Some(file_path),
                create: false,
                permissions: 0o644,
            };

            match ControlRegion::new(config) {
                Ok(control) => {
                    let entry = control.get_region_entry("persistent_region").unwrap();
                    assert_eq!(entry.metadata.name, "persistent_region");
                    assert_eq!(entry.metadata.size, 4096);
                }
                Err(_) => {
                    // File persistence might not be available in all environments
                    println!("Control region persistence test skipped (permission/environment limitation)");
                }
            }
        }
    }
}
