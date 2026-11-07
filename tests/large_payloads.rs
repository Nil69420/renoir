use renoir::large_payloads::{
    blob::{content_types, BlobDescriptor, BlobHeader, BlobManager},
    chunking::ChunkingStrategy,
    reclamation::ReclamationPolicy,
    ros2_types::{ImageHeader, LaserScanHeader, PointCloudHeader, ROS2MessageType},
};
use renoir::{
    memory::{BackingType, RegionConfig, SharedMemoryRegion},
    shared_pools::SharedBufferPoolManager,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Helper function to create test shared memory setup - simplified to avoid segfaults
pub fn create_test_pool_manager(
) -> Option<(Arc<SharedBufferPoolManager>, renoir::shared_pools::PoolId)> {
    // Use a unique name for each test run to avoid conflicts
    let test_name = format!("test_large_payloads_{}", std::process::id());
    let temp_file = format!("/tmp/{}", test_name);

    let region = Arc::new(
        SharedMemoryRegion::new(RegionConfig {
            name: test_name,
            size: 1024 * 1024, // 1MB - smaller region to avoid issues
            backing_type: BackingType::FileBacked,
            file_path: Some(PathBuf::from(&temp_file)),
            create: true,
            permissions: 0o644,
        })
        .ok()?,
    );

    let pool_manager = Arc::new(SharedBufferPoolManager::new());
    let pool_id = pool_manager
        .create_pool(
            "test_pool".to_string(),
            4 * 1024, // 4KB buffers
            10,       // Only 10 buffers to stay within limits
            region,
        )
        .ok()?;

    Some((pool_manager, pool_id))
}

#[cfg(test)]
mod blob_tests {
    use super::*;

    #[test]
    fn test_blob_header_creation() {
        let manager = BlobManager::new();
        let data = b"Hello, world!";

        let header = manager.create_header(content_types::IMAGE, data);

        // Use local variables to avoid packed field references
        let magic = header.magic;
        let version = header.version;
        let content_type = header.content_type;
        let payload_size = header.payload_size;
        let checksum = header.checksum;

        assert_eq!(magic, renoir::large_payloads::blob::BLOB_MAGIC);
        assert_eq!(version, renoir::large_payloads::blob::BLOB_VERSION);
        assert_eq!(content_type, content_types::IMAGE);
        assert_eq!(payload_size, data.len() as u64);
        assert!(checksum != 0); // Should have calculated checksum
    }

    #[test]
    fn test_blob_manager_validation() {
        let manager = BlobManager::new();
        let data = b"test data";

        let valid_header = manager.create_header(content_types::POINT_CLOUD, data);
        assert!(manager.validate_blob(&valid_header, data).is_ok());

        // Test invalid magic (create new header with invalid magic)
        let mut invalid_header = valid_header;
        invalid_header.magic = 0xDEADBEEF;
        assert!(manager.validate_blob(&invalid_header, data).is_err());
    }

    #[test]
    fn test_blob_descriptor() {
        let header = BlobHeader::new(content_types::LASER_SCAN, 1024);
        let desc = BlobDescriptor::new(1, 42, header);

        assert_eq!(desc.pool_id, 1);
        assert_eq!(desc.buffer_handle, 42);
        // Use local variable to avoid packed field reference
        let payload_size = desc.header.payload_size;
        assert_eq!(payload_size, 1024);
        assert_eq!(desc.ref_count(), 1);

        // Test reference counting
        desc.add_ref();
        assert_eq!(desc.ref_count(), 2);

        assert!(!desc.release()); // Should not be ready for reclamation yet
        assert_eq!(desc.ref_count(), 1);

        assert!(desc.release()); // Now should be ready for reclamation
        assert_eq!(desc.ref_count(), 0);
    }
}

#[cfg(test)]
mod chunking_tests {
    use super::*;

    #[test]
    fn test_chunking_strategy_creation() {
        let strategy = ChunkingStrategy::default();

        // Just test that we can create a strategy - actual chunking logic is internal
        match strategy {
            ChunkingStrategy::FixedSize(size) => {
                assert_eq!(size, 1024 * 1024); // 1MB default
            }
            ChunkingStrategy::Adaptive { min_size, max_size } => {
                assert!(min_size <= max_size);
            }
        }
    }
}

#[cfg(test)]
mod reclamation_tests {
    use super::*;

    #[test]
    fn test_reclamation_policy_creation() {
        let policy = ReclamationPolicy::default();
        // Test basic policy creation without shared memory

        // Check default values exist
        let max_age = policy.max_age;
        assert!(max_age > std::time::Duration::from_secs(0));
    }
}

#[cfg(test)]
mod ros2_types_tests {
    use super::*;

    #[test]
    fn test_image_header() {
        let header = ImageHeader::new(1920, 1080, "rgb8", 1920 * 3);

        // Use local variables to avoid packed field references
        let width = header.width;
        let height = header.height;
        let step = header.step;

        assert_eq!(width, 1920);
        assert_eq!(height, 1080);
        assert_eq!(header.encoding_str(), "rgb8");
        assert_eq!(step, 1920 * 3);
        assert_eq!(header.data_size(), 1920 * 1080 * 3);
    }

    #[test]
    fn test_point_cloud_header() {
        let header = PointCloudHeader::new(100000, 16, 640, 480, true);

        // Use local variables to avoid packed field references
        let point_count = header.point_count;
        let point_step = header.point_step;
        let width = header.width;
        let height = header.height;

        assert_eq!(point_count, 100000);
        assert_eq!(point_step, 16);
        assert_eq!(width, 640);
        assert_eq!(height, 480);
        assert_eq!(header.data_size(), 100000 * 16);
        assert!(header.is_organized());
    }

    #[test]
    fn test_laser_scan_header() {
        let header = LaserScanHeader::new(
            -std::f32::consts::PI,
            std::f32::consts::PI,
            0.01,
            0.1,
            10.0,
            628,
            628,
        );

        // Use local variables to avoid packed field references
        let range_count = header.range_count;
        let intensity_count = header.intensity_count;

        assert_eq!(range_count, 628);
        assert_eq!(intensity_count, 628);
        assert_eq!(header.data_size(), 628 * 4 + 628 * 4); // ranges + intensities
    }

    #[test]
    fn test_ros2_message_types() {
        assert_eq!(ROS2MessageType::Image.content_type(), content_types::IMAGE);
        assert_eq!(
            ROS2MessageType::PointCloud2.content_type(),
            content_types::POINT_CLOUD
        );
        assert_eq!(
            ROS2MessageType::LaserScan.content_type(),
            content_types::LASER_SCAN
        );

        // Test typical sizes
        assert!(ROS2MessageType::Image.typical_size_class() > 1_000_000); // > 1MB
        assert!(ROS2MessageType::PointCloud2.typical_size_class() > 100_000); // > 100KB
    }

    #[test]
    fn test_ros2_message_manager_basic() {
        let _strategy = ChunkingStrategy::default();
    }

    #[test]
    fn test_process_chunk_rejects_corrupted_chunk_size() {
        use renoir::large_payloads::chunking::{ChunkDescriptor, ChunkManager, ChunkingStrategy};
        use renoir::shared_pools::SharedBufferPoolManager;

        let pool_manager = Arc::new(SharedBufferPoolManager::new());
        let chunk_manager = ChunkManager::new(pool_manager, ChunkingStrategy::FixedSize(64));

        // Build a buffer holding a descriptor whose chunk_size is way larger
        // than the remaining data — previously indexed out of bounds (panic)
        let mut descriptor = ChunkDescriptor::new(1, 0, 1, 8, 0);
        descriptor.chunk_size = u32::MAX;

        let mut chunk_buffer = vec![0u8; ChunkDescriptor::SIZE + 8];
        unsafe {
            std::ptr::copy_nonoverlapping(
                &descriptor as *const ChunkDescriptor as *const u8,
                chunk_buffer.as_mut_ptr(),
                ChunkDescriptor::SIZE,
            );
        }

        let result = chunk_manager.process_chunk(&chunk_buffer);
        assert!(
            result.is_err(),
            "corrupted chunk_size must be rejected, not panic or slice out of bounds"
        );

        let truncated = vec![0u8; ChunkDescriptor::SIZE + 3];
        let result = chunk_manager.process_chunk(&truncated);
        assert!(result.is_err());
    }

    #[test]
    fn test_chunked_blob_round_trip_and_no_leak() {
        use renoir::large_payloads::chunking::ChunkingStrategy;
        use renoir::large_payloads::ros2_types::ROS2MessageManager;
        use std::path::PathBuf;

        // 1KB buffers, 256-byte chunks: forces chunking for large payloads
        let test_name = format!("test_chunked_roundtrip_{}", std::process::id());
        let temp_file = format!("/tmp/{}", test_name);
        let region = Arc::new(
            SharedMemoryRegion::new(RegionConfig {
                name: test_name,
                size: 1024 * 1024,
                backing_type: BackingType::FileBacked,
                file_path: Some(PathBuf::from(&temp_file)),
                create: true,
                permissions: 0o644,
            })
            .unwrap(),
        );

        let pool_manager = Arc::new(SharedBufferPoolManager::new());
        let pool_id = pool_manager
            .create_pool("chunk_test_pool".to_string(), 1024, 32, region)
            .unwrap();

        let manager =
            ROS2MessageManager::new(pool_manager.clone(), ChunkingStrategy::FixedSize(256));

        let (_, _, baseline_active, _) = pool_manager.pool_stats(pool_id).unwrap();

        // Payload large enough to span multiple chunks (> 1KB - BlobHeader)
        let payload = vec![0xABu8; 5000];

        let image = manager
            .create_image_message(50, 100, "mono8", &payload, pool_id)
            .unwrap();
        assert!(
            image.blob.header.is_chunked(),
            "5000-byte payload must be chunked"
        );
        assert!(!image.blob.chunk_descriptors.is_empty());
        let chunk_count = image.blob.header.chunk_count;
        assert_eq!(image.blob.chunk_descriptors.len() as u32, chunk_count);

        let (_, _, active_before, _) = pool_manager.pool_stats(pool_id).unwrap();
        assert!(active_before > baseline_active);

        let reassembled = manager.read_blob_payload(&image.blob).unwrap();
        assert_eq!(
            reassembled.len(),
            payload.len() + renoir::large_payloads::ros2_types::ImageHeader::SIZE,
            "reassembled payload must include image header"
        );
        assert_eq!(
            &reassembled[renoir::large_payloads::ros2_types::ImageHeader::SIZE..],
            &payload[..],
            "reassembled image data must equal the original payload"
        );

        manager.release_blob(&image.blob).unwrap();

        let (_, _, active_after, _) = pool_manager.pool_stats(pool_id).unwrap();
        assert_eq!(
            active_after, baseline_active,
            "all chunk buffers must be returned to the pool after release"
        );
    }

    #[test]
    fn test_chunk_write_rejects_undersized_pool() {
        use renoir::large_payloads::blob::{content_types, BlobManager};
        use renoir::large_payloads::chunking::{ChunkManager, ChunkingStrategy};
        use std::path::PathBuf;

        // Buffer pool with buffers smaller than a chunk's footprint
        // (ChunkDescriptor::SIZE + chunk data)
        let test_name = format!("test_undersized_pool_{}", std::process::id());
        let temp_file = format!("/tmp/{}", test_name);
        let region = Arc::new(
            SharedMemoryRegion::new(RegionConfig {
                name: test_name,
                size: 1024 * 1024,
                backing_type: BackingType::FileBacked,
                file_path: Some(PathBuf::from(&temp_file)),
                create: true,
                permissions: 0o644,
            })
            .unwrap(),
        );

        let pool_manager = Arc::new(SharedBufferPoolManager::new());
        let pool_id = pool_manager
            .create_pool("tiny_pool".to_string(), 64, 16, region)
            .unwrap();

        let chunk_manager =
            ChunkManager::new(pool_manager.clone(), ChunkingStrategy::FixedSize(256));
        let blob_manager = BlobManager::new();
        let payload = vec![0xCDu8; 1024];
        let header = blob_manager.create_header(content_types::CUSTOM_BINARY, &payload);

        let result = chunk_manager.chunk_payload(&payload, &header, pool_id);
        assert!(
            result.is_err(),
            "chunk write into a 64-byte buffer with 256-byte chunks must be rejected, \
             not write out of bounds"
        );
    }
}
