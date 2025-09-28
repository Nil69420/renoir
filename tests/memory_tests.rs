//! Integration tests for memory management components

use tempfile::TempDir;
use renoir::{
    memory::{BackingType, RegionConfig, SharedMemoryRegion, SharedMemoryManager},
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_config_default() {
        let config = RegionConfig::default();
        assert_eq!(config.backing_type, BackingType::FileBacked);
        assert!(config.create);
        assert_eq!(config.permissions, 0o644);
    }

    #[test]
    fn test_region_config_builder() {
        let config = RegionConfig::new("test", 4096)
            .with_backing_type(BackingType::FileBacked)
            .with_create(true)
            .with_permissions(0o600);
        
        assert_eq!(config.name, "test");
        assert_eq!(config.size, 4096);
        assert_eq!(config.backing_type, BackingType::FileBacked);
        assert!(config.create);
        assert_eq!(config.permissions, 0o600);
    }

    #[test]
    fn test_region_config_validation() {
        let mut config = RegionConfig::default();
        
        // Empty name should fail
        assert!(config.validate().is_err());
        
        config.name = "test".to_string();
        // Zero size should fail
        assert!(config.validate().is_err());
        
        config.size = 4096;
        // Valid config should pass
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_create_file_backed_region() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "test_region".to_string(),
            size: 4096,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("test_shm")),
            create: true,
            permissions: 0o644,
        };

        let region = SharedMemoryRegion::new(config).unwrap();
        assert_eq!(region.name(), "test_region");
        assert_eq!(region.size(), 4096);
        assert!(region.is_file_backed());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_create_memfd_region() {
        let config = RegionConfig {
            name: "test_memfd".to_string(),
            size: 4096,
            backing_type: BackingType::MemFd,
            file_path: None,
            create: true,
            permissions: 0o644,
        };

        let region = SharedMemoryRegion::new(config).unwrap();
        assert_eq!(region.name(), "test_memfd");
        assert_eq!(region.size(), 4096);
        assert!(region.is_memfd_backed());
    }

    #[test]
    fn test_region_memory_operations() {
        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "test_memory".to_string(),
            size: 4096,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("test_mem")),
            create: true,
            permissions: 0o644,
        };

        let mut region = SharedMemoryRegion::new(config).unwrap();
        
        // Test mutable access
        let slice = region.as_mut_slice();
        slice[0] = 42;
        slice[1] = 24;
        
        // Test read-only access
        let read_slice = region.as_slice();
        assert_eq!(read_slice[0], 42);
        assert_eq!(read_slice[1], 24);
        
        // Test pointer access
        let ptr = region.as_ptr::<u8>();
        unsafe {
            assert_eq!(*ptr, 42);
        }
    }

    #[test]
    fn test_shared_memory_manager() {
        let manager = SharedMemoryManager::new();
        assert_eq!(manager.region_count(), 0);
        assert!(manager.list_regions().is_empty());

        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "test_region".to_string(),
            size: 4096,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("test_shm")),
            create: true,
            permissions: 0o644,
        };

        // Create region
        let _region = manager.create_region(config).unwrap();
        assert_eq!(manager.region_count(), 1);
        assert!(manager.has_region("test_region"));
        
        // Get region
        let retrieved = manager.get_region("test_region").unwrap();
        assert_eq!(retrieved.name(), "test_region");

        // List regions
        let regions = manager.list_regions();
        assert_eq!(regions.len(), 1);
        assert!(regions.contains(&"test_region".to_string()));

        // Remove region
        manager.remove_region("test_region").unwrap();
        assert_eq!(manager.region_count(), 0);
        assert!(!manager.has_region("test_region"));
    }

    #[test]
    fn test_manager_memory_stats() {
        let manager = SharedMemoryManager::new();
        
        let temp_dir = TempDir::new().unwrap();
        let config1 = RegionConfig {
            name: "region1".to_string(),
            size: 4096,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("region1")),
            create: true,
            permissions: 0o644,
        };

        let config2 = RegionConfig {
            name: "region2".to_string(),
            size: 8192,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("region2")),
            create: true,
            permissions: 0o644,
        };

        manager.create_region(config1).unwrap();
        manager.create_region(config2).unwrap();

        // Test memory statistics
        assert_eq!(manager.total_memory_usage(), 4096 + 8192);
        
        let stats = manager.memory_stats();
        assert_eq!(stats.len(), 2);
        
        let region1_stats = stats.iter().find(|s| s.name == "region1").unwrap();
        assert_eq!(region1_stats.size, 4096);
        
        let region2_stats = stats.iter().find(|s| s.name == "region2").unwrap();
        assert_eq!(region2_stats.size, 8192);
    }

    #[test]
    fn test_manager_clear() {
        let manager = SharedMemoryManager::new();
        
        let temp_dir = TempDir::new().unwrap();
        for i in 0..3 {
            let config = RegionConfig {
                name: format!("region{}", i),
                size: 4096,
                backing_type: BackingType::FileBacked,
                file_path: Some(temp_dir.path().join(format!("region{}", i))),
                create: true,
                permissions: 0o644,
            };
            manager.create_region(config).unwrap();
        }

        assert_eq!(manager.region_count(), 3);
        
        manager.clear().unwrap();
        assert_eq!(manager.region_count(), 0);
    }
}