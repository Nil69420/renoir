//! Tests for shared buffer pools

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    
    use renoir::{
        shared_pools::{SharedBufferPoolManager, BufferPoolRegistry},
        memory::{SharedMemoryRegion, RegionConfig, BackingType, SharedMemoryManager},
        topic::MessageDescriptor,
    };

    fn create_test_region() -> std::sync::Arc<SharedMemoryRegion> {
        let manager = SharedMemoryManager::new();
        let config = RegionConfig {
            name: "test_shared_pool".to_string(),
            size: 1024 * 1024, // 1MB
            backing_type: BackingType::MemFd,
            file_path: None,
            create: true,
            permissions: 0o600,
        };
        manager.create_region(config).expect("Failed to create region")
    }

    #[test]
    fn test_shared_pool_manager_creation() {
        let manager = SharedBufferPoolManager::new();
        let stats = manager.global_stats();
        assert_eq!(stats.pools_created.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_pool_creation_and_buffer_operations() {
        let manager = SharedBufferPoolManager::new();
        let region = create_test_region();

        // Create a pool
        let pool_id = manager.create_pool(
            "test_pool".to_string(),
            1024,
            10,
            region,
        ).unwrap();

        // Get a buffer
        let descriptor = manager.get_buffer(pool_id).unwrap();
        assert_eq!(descriptor.pool_id, pool_id);
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 1);

        // Get buffer data
        let buffer = manager.get_buffer_data(&descriptor).unwrap();
        assert_eq!(buffer.capacity(), 1024);

        // Return the buffer
        manager.return_buffer(&descriptor).unwrap();

        // Verify pool stats
        let (allocated, returned, active, _peak) = manager.pool_stats(pool_id).unwrap();
        assert_eq!(allocated, 1);
        assert_eq!(returned, 1);
        assert_eq!(active, 0);
    }

    #[test]
    fn test_buffer_pool_registry() {
        let region = create_test_region();
        let registry = BufferPoolRegistry::new(region);

        // Get pools for different size classes
        let pool_1k = registry.get_pool_for_size(1000).unwrap();
        let pool_2k = registry.get_pool_for_size(2000).unwrap();
        let pool_1k_again = registry.get_pool_for_size(1024).unwrap();

        // Same size class should return same pool
        assert_eq!(pool_1k, pool_1k_again);
        // Different size classes should return different pools
        assert_ne!(pool_1k, pool_2k);

        // Test buffer operations
        let descriptor = registry.get_buffer_for_payload(500).unwrap();
        let buffer_data = registry.get_buffer_data(&descriptor).unwrap();
        assert!(buffer_data.capacity() >= 500);

        registry.return_buffer(&descriptor).unwrap();
    }

    #[test]
    fn test_message_descriptor_ref_counting() {
        let descriptor = MessageDescriptor::new(1, 1, 1024);
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 1);

        // Add reference
        descriptor.add_ref();
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 2);

        // Release one reference
        assert!(!descriptor.release());
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 1);

        // Release final reference
        assert!(descriptor.release());
        assert_eq!(descriptor.ref_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_registry_cleanup() {
        let region = create_test_region();
        let registry = BufferPoolRegistry::new(region);

        // Create some buffers and return them
        let descriptor1 = registry.get_buffer_for_payload(512).unwrap();
        let descriptor2 = registry.get_buffer_for_payload(1024).unwrap();
        
        registry.return_buffer(&descriptor1).unwrap();
        registry.return_buffer(&descriptor2).unwrap();

        // Should be able to cleanup unused pools
        let _removed = registry.cleanup_unused_pools().unwrap();
        // Number of pools removed may vary depending on implementation
    }
}