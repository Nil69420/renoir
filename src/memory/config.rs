//! Configuration types for shared memory regions

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Types of shared memory backing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackingType {
    /// File-backed shared memory
    FileBacked,
    /// Anonymous memory file descriptor (Linux-specific)
    #[cfg(target_os = "linux")]
    MemFd,
}

impl Default for BackingType {
    fn default() -> Self {
        Self::FileBacked
    }
}

impl BackingType {
    /// Check if this backing type is supported on the current platform
    pub fn is_supported(&self) -> bool {
        match self {
            BackingType::FileBacked => true,
            #[cfg(target_os = "linux")]
            BackingType::MemFd => true,
            #[cfg(not(target_os = "linux"))]
            BackingType::MemFd => false,
        }
    }

    /// Get a human-readable name for the backing type
    pub fn name(&self) -> &'static str {
        match self {
            BackingType::FileBacked => "file-backed",
            #[cfg(target_os = "linux")]
            BackingType::MemFd => "memfd",
        }
    }
}

/// Configuration for creating shared memory regions
#[derive(Debug, Clone)]
pub struct RegionConfig {
    /// Name of the shared memory region
    pub name: String,
    /// Total size of the region in bytes
    pub size: usize,
    /// Backing type for the shared memory
    pub backing_type: BackingType,
    /// Optional file path for file-backed regions
    pub file_path: Option<PathBuf>,
    /// Whether to create the region if it doesn't exist
    pub create: bool,
    /// Permissions for the region (Unix permissions)
    pub permissions: u32,
}

impl Default for RegionConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            size: 0,
            backing_type: BackingType::default(),
            file_path: None,
            create: true,
            permissions: 0o644,
        }
    }
}

impl RegionConfig {
    /// Create a new region configuration
    pub fn new(name: impl Into<String>, size: usize) -> Self {
        Self {
            name: name.into(),
            size,
            ..Default::default()
        }
    }

    /// Set the backing type
    pub fn with_backing_type(mut self, backing_type: BackingType) -> Self {
        self.backing_type = backing_type;
        self
    }

    /// Set the file path for file-backed regions
    pub fn with_file_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Set whether to create the region if it doesn't exist
    pub fn with_create(mut self, create: bool) -> Self {
        self.create = create;
        self
    }

    /// Set the permissions for the region
    pub fn with_permissions(mut self, permissions: u32) -> Self {
        self.permissions = permissions;
        self
    }

    /// Validate the configuration
    pub fn validate(&self) -> crate::Result<()> {
        use crate::error::RenoirError;

        if self.name.is_empty() {
            return Err(RenoirError::invalid_parameter(
                "name",
                "Region name cannot be empty",
            ));
        }

        if self.size == 0 {
            return Err(RenoirError::invalid_parameter(
                "size",
                "Region size must be greater than 0",
            ));
        }

        if !self.backing_type.is_supported() {
            return Err(RenoirError::invalid_parameter(
                "backing_type",
                &format!(
                    "Backing type {} is not supported on this platform",
                    self.backing_type.name()
                ),
            ));
        }

        // For file-backed regions, ensure we have a path if not creating
        if self.backing_type == BackingType::FileBacked && !self.create && self.file_path.is_none()
        {
            return Err(RenoirError::invalid_parameter(
                "file_path",
                "File path must be specified for existing file-backed regions",
            ));
        }

        Ok(())
    }

    /// Get the default file path for this region
    pub fn default_file_path(&self) -> PathBuf {
        self.file_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("/tmp/renoir_{}", self.name)))
    }
}
