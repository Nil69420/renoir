//! ROS2-specific message types for variable-sized payloads
//!
//! Specialized handling for common ROS2 message types that can have
//! highly variable sizes: images, point clouds, laser scans, etc.

use crate::error::{RenoirError, Result};
use crate::shared_pools::{PoolId, SharedBufferPoolManager};
use std::sync::Arc;

use super::blob::{content_types, BlobDescriptor, BlobHeader, BlobManager};
use super::chunking::{ChunkManager, ChunkingStrategy};

/// Common ROS2 message type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ROS2MessageType {
    /// sensor_msgs/Image
    Image,
    /// sensor_msgs/PointCloud2  
    PointCloud2,
    /// sensor_msgs/LaserScan
    LaserScan,
    /// nav_msgs/OccupancyGrid
    OccupancyGrid,
    /// geometry_msgs/PolygonStamped
    Polygon,
    /// Custom binary data
    Custom(u32),
}

impl ROS2MessageType {
    /// Get content type code for blob header
    pub fn content_type(&self) -> u32 {
        match self {
            Self::Image => content_types::IMAGE,
            Self::PointCloud2 => content_types::POINT_CLOUD,
            Self::LaserScan => content_types::LASER_SCAN,
            Self::OccupancyGrid => content_types::OCCUPANCY_GRID,
            Self::Polygon => content_types::POLYGON,
            Self::Custom(code) => *code,
        }
    }

    /// Get typical size class for pool allocation
    pub fn typical_size_class(&self) -> usize {
        match self {
            Self::Image => 1920 * 1080 * 3,     // HD RGB image
            Self::PointCloud2 => 100_000 * 16,  // 100k points with XYZ + intensity
            Self::LaserScan => 1440 * 4,        // 1440 points (0.25° resolution)
            Self::OccupancyGrid => 1000 * 1000, // 1000x1000 grid
            Self::Polygon => 1000 * 8,          // 1000 points * (x,y)
            Self::Custom(_) => 64 * 1024,       // 64KB default
        }
    }

    /// Check if this message type typically needs chunking
    pub fn needs_chunking_typically(&self, max_buffer_size: usize) -> bool {
        self.typical_size_class() > max_buffer_size
    }
}

/// Image message structure (sensor_msgs/Image equivalent)
#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct ImageHeader {
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
    /// Pixel format encoding (e.g., "rgb8", "bgr8", "mono8")
    pub encoding: [u8; 16], // Fixed-size encoding string
    /// Bytes per row (width * channels * bytes_per_channel)
    pub step: u32,
    /// Whether data is big-endian (1) or little-endian (0)
    pub is_bigendian: u8,
    /// Reserved for alignment
    pub reserved: [u8; 3],
}

impl ImageHeader {
    /// Size of image header in bytes
    pub const SIZE: usize = std::mem::size_of::<ImageHeader>();

    /// Create new image header
    pub fn new(width: u32, height: u32, encoding: &str, step: u32) -> Self {
        let mut encoding_bytes = [0u8; 16];
        let encoding_str = encoding.as_bytes();
        let copy_len = std::cmp::min(encoding_str.len(), 15); // Leave space for null terminator
        encoding_bytes[..copy_len].copy_from_slice(&encoding_str[..copy_len]);

        Self {
            width,
            height,
            encoding: encoding_bytes,
            step,
            is_bigendian: 0, // Assume little-endian
            reserved: [0; 3],
        }
    }

    /// Get encoding as string
    pub fn encoding_str(&self) -> &str {
        // Find null terminator
        let len = self.encoding.iter().position(|&b| b == 0).unwrap_or(16);
        std::str::from_utf8(&self.encoding[..len]).unwrap_or("unknown")
    }

    /// Calculate expected data size
    pub fn data_size(&self) -> usize {
        (self.height * self.step) as usize
    }
}

/// Point cloud message structure (sensor_msgs/PointCloud2 equivalent)
#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct PointCloudHeader {
    /// Number of points
    pub point_count: u32,
    /// Size of each point in bytes
    pub point_step: u32,
    /// Total row size in bytes (point_count * point_step)
    pub row_step: u32,
    /// Point cloud width (for organized clouds)
    pub width: u32,
    /// Point cloud height (for organized clouds, 1 for unorganized)
    pub height: u32,
    /// Whether cloud is dense (no invalid points)
    pub is_dense: u8,
    /// Reserved for alignment
    pub reserved: [u8; 7],
}

impl PointCloudHeader {
    /// Size of point cloud header in bytes
    pub const SIZE: usize = std::mem::size_of::<PointCloudHeader>();

    /// Create new point cloud header
    pub fn new(point_count: u32, point_step: u32, width: u32, height: u32, is_dense: bool) -> Self {
        Self {
            point_count,
            point_step,
            row_step: point_count * point_step,
            width,
            height,
            is_dense: if is_dense { 1 } else { 0 },
            reserved: [0; 7],
        }
    }

    /// Calculate expected data size
    pub fn data_size(&self) -> usize {
        self.row_step as usize
    }

    /// Check if cloud is organized
    pub fn is_organized(&self) -> bool {
        self.width > 1 && self.height > 1
    }
}

/// Laser scan message structure (sensor_msgs/LaserScan equivalent)
#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct LaserScanHeader {
    /// Minimum angle (radians)
    pub angle_min: f32,
    /// Maximum angle (radians)
    pub angle_max: f32,
    /// Angular resolution (radians)
    pub angle_increment: f32,
    /// Minimum range (meters)
    pub range_min: f32,
    /// Maximum range (meters)
    pub range_max: f32,
    /// Number of range measurements
    pub range_count: u32,
    /// Number of intensity measurements (may be 0)
    pub intensity_count: u32,
    /// Reserved for alignment
    pub reserved: [u8; 8],
}

impl LaserScanHeader {
    /// Size of laser scan header in bytes
    pub const SIZE: usize = std::mem::size_of::<LaserScanHeader>();

    /// Create new laser scan header
    pub fn new(
        angle_min: f32,
        angle_max: f32,
        angle_increment: f32,
        range_min: f32,
        range_max: f32,
        range_count: u32,
        intensity_count: u32,
    ) -> Self {
        Self {
            angle_min,
            angle_max,
            angle_increment,
            range_min,
            range_max,
            range_count,
            intensity_count,
            reserved: [0; 8],
        }
    }

    /// Calculate expected data size
    pub fn data_size(&self) -> usize {
        (self.range_count * 4) as usize + // ranges as f32
        (self.intensity_count * 4) as usize // intensities as f32
    }
}

/// Generic ROS2 message wrapper for variable-sized payloads
#[derive(Debug)]
pub struct ImageMessage {
    /// Image header
    pub header: ImageHeader,
    /// Blob descriptor for image data
    pub blob: BlobDescriptor,
}

/// Point cloud message wrapper
#[derive(Debug)]
pub struct PointCloudMessage {
    /// Point cloud header
    pub header: PointCloudHeader,
    /// Blob descriptor for point data
    pub blob: BlobDescriptor,
}

/// Laser scan message wrapper
#[derive(Debug)]
pub struct LaserScanMessage {
    /// Laser scan header
    pub header: LaserScanHeader,
    /// Blob descriptor for range/intensity data
    pub blob: BlobDescriptor,
}

/// Manager for ROS2 variable-sized messages
#[derive(Debug)]
pub struct ROS2MessageManager {
    /// Shared buffer pool manager
    pool_manager: Arc<SharedBufferPoolManager>,
    /// Blob manager for headers
    blob_manager: BlobManager,
    /// Chunk manager for large messages
    chunk_manager: ChunkManager,
}

impl ROS2MessageManager {
    /// Create new ROS2 message manager
    pub fn new(
        pool_manager: Arc<SharedBufferPoolManager>,
        chunking_strategy: ChunkingStrategy,
    ) -> Self {
        let chunk_manager = ChunkManager::new(pool_manager.clone(), chunking_strategy);

        Self {
            pool_manager,
            blob_manager: BlobManager::new(),
            chunk_manager,
        }
    }

    /// Create an image message
    pub fn create_image_message(
        &self,
        width: u32,
        height: u32,
        encoding: &str,
        image_data: &[u8],
        pool_id: PoolId,
    ) -> Result<ImageMessage> {
        // Calculate step size based on encoding
        let channels = match encoding {
            "mono8" => 1,
            "rgb8" | "bgr8" => 3,
            "rgba8" | "bgra8" => 4,
            "mono16" => 2,
            "rgb16" | "bgr16" => 6,
            _ => {
                return Err(RenoirError::invalid_parameter(
                    "encoding",
                    "Unsupported image encoding",
                ))
            }
        };
        let step = width * channels;

        let header = ImageHeader::new(width, height, encoding, step);

        // Validate data size
        let expected_size = header.data_size();
        if image_data.len() != expected_size {
            return Err(RenoirError::invalid_parameter(
                "image_data",
                "Image data size doesn't match header dimensions",
            ));
        }

        // Create combined payload (header + data)
        let mut combined_payload = Vec::with_capacity(ImageHeader::SIZE + image_data.len());
        combined_payload.extend_from_slice(unsafe {
            std::slice::from_raw_parts(
                &header as *const ImageHeader as *const u8,
                ImageHeader::SIZE,
            )
        });
        combined_payload.extend_from_slice(image_data);

        // Create blob
        let blob_header = self
            .blob_manager
            .create_header(ROS2MessageType::Image.content_type(), &combined_payload);
        let blob = self.store_blob(blob_header, &combined_payload, pool_id)?;

        Ok(ImageMessage { header, blob })
    }

    /// Create a point cloud message
    pub fn create_point_cloud_message(
        &self,
        point_count: u32,
        point_step: u32,
        width: u32,
        height: u32,
        is_dense: bool,
        point_data: &[u8],
        pool_id: PoolId,
    ) -> Result<PointCloudMessage> {
        let header = PointCloudHeader::new(point_count, point_step, width, height, is_dense);

        // Validate data size
        let expected_size = header.data_size();
        if point_data.len() != expected_size {
            return Err(RenoirError::invalid_parameter(
                "point_data",
                "Point cloud data size doesn't match header",
            ));
        }

        // Create combined payload
        let mut combined_payload = Vec::with_capacity(PointCloudHeader::SIZE + point_data.len());
        combined_payload.extend_from_slice(unsafe {
            std::slice::from_raw_parts(
                &header as *const PointCloudHeader as *const u8,
                PointCloudHeader::SIZE,
            )
        });
        combined_payload.extend_from_slice(point_data);

        let blob_header = self.blob_manager.create_header(
            ROS2MessageType::PointCloud2.content_type(),
            &combined_payload,
        );
        let blob = self.store_blob(blob_header, &combined_payload, pool_id)?;

        Ok(PointCloudMessage { header, blob })
    }

    /// Create a laser scan message
    pub fn create_laser_scan_message(
        &self,
        angle_min: f32,
        angle_max: f32,
        angle_increment: f32,
        range_min: f32,
        range_max: f32,
        ranges: &[f32],
        intensities: &[f32],
        pool_id: PoolId,
    ) -> Result<LaserScanMessage> {
        let header = LaserScanHeader::new(
            angle_min,
            angle_max,
            angle_increment,
            range_min,
            range_max,
            ranges.len() as u32,
            intensities.len() as u32,
        );

        // Create combined data payload
        let mut data_payload = Vec::with_capacity(header.data_size());

        // Add ranges as bytes
        for &range in ranges {
            data_payload.extend_from_slice(&range.to_le_bytes());
        }

        // Add intensities as bytes
        for &intensity in intensities {
            data_payload.extend_from_slice(&intensity.to_le_bytes());
        }

        // Create combined payload (header + data)
        let mut combined_payload = Vec::with_capacity(LaserScanHeader::SIZE + data_payload.len());
        combined_payload.extend_from_slice(unsafe {
            std::slice::from_raw_parts(
                &header as *const LaserScanHeader as *const u8,
                LaserScanHeader::SIZE,
            )
        });
        combined_payload.extend_from_slice(&data_payload);

        let blob_header = self
            .blob_manager
            .create_header(ROS2MessageType::LaserScan.content_type(), &combined_payload);
        let blob = self.store_blob(blob_header, &combined_payload, pool_id)?;

        Ok(LaserScanMessage { header, blob })
    }

    /// Store blob in buffer pool (with potential chunking)
    fn store_blob(
        &self,
        blob_header: BlobHeader,
        payload: &[u8],
        pool_id: PoolId,
    ) -> Result<BlobDescriptor> {
        // Get buffer to check size limits
        let test_buffer = self.pool_manager.get_buffer(pool_id)?;
        let buffer_data = self.pool_manager.get_buffer_data(&test_buffer)?;
        let max_buffer_size = buffer_data.as_slice().len() - BlobHeader::SIZE;

        // Return test buffer
        self.pool_manager.return_buffer(&test_buffer)?;

        if payload.len() <= max_buffer_size {
            // Fits in single buffer
            self.store_single_blob(blob_header, payload, pool_id)
        } else {
            // Needs chunking
            self.store_chunked_blob(blob_header, payload, pool_id)
        }
    }

    /// Store single blob in buffer
    fn store_single_blob(
        &self,
        blob_header: BlobHeader,
        payload: &[u8],
        pool_id: PoolId,
    ) -> Result<BlobDescriptor> {
        let buffer_desc = self.pool_manager.get_buffer(pool_id)?;
        let buffer_data = self.pool_manager.get_buffer_data(&buffer_desc)?;

        // Write blob header + payload to buffer
        unsafe {
            let buffer_ptr = buffer_data.as_ptr() as *mut u8;

            // Write header
            std::ptr::copy_nonoverlapping(
                &blob_header as *const BlobHeader as *const u8,
                buffer_ptr,
                BlobHeader::SIZE,
            );

            // Write payload
            std::ptr::copy_nonoverlapping(
                payload.as_ptr(),
                buffer_ptr.add(BlobHeader::SIZE),
                payload.len(),
            );
        }

        Ok(BlobDescriptor::new(
            pool_id,
            buffer_desc.buffer_handle,
            blob_header,
        ))
    }

    /// Store chunked blob across multiple buffers
    fn store_chunked_blob(
        &self,
        mut blob_header: BlobHeader,
        payload: &[u8],
        pool_id: PoolId,
    ) -> Result<BlobDescriptor> {
        // Update blob header for chunking
        let sequence_id = self.blob_manager.next_sequence_id();
        blob_header.sequence_id = sequence_id;

        // Create chunks
        let _chunk_descriptors =
            self.chunk_manager
                .chunk_payload(payload, &blob_header, pool_id)?;

        // For chunked messages, we store the blob header separately
        // and use the sequence_id to reassemble
        self.store_single_blob(blob_header, &[], pool_id)
    }
}
