//! Metadata types and data structures

use std::time::SystemTime;
use serde::{Deserialize, Serialize};

use crate::memory::BackingType;

/// Metadata for a shared memory region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionMetadata {
    /// Name of the region
    pub name: String,
    /// Size in bytes
    pub size: usize,
    /// Type of backing storage
    pub backing_type: BackingType,
    /// Creation timestamp
    pub created_at: SystemTime,
    /// Version number for compatibility checking
    pub version: u64,
}

impl RegionMetadata {
    /// Create new metadata
    pub fn new(
        name: String,
        size: usize,
        backing_type: BackingType,
    ) -> Self {
        Self {
            name,
            size,
            backing_type,
            created_at: SystemTime::now(),
            version: 1,
        }
    }

    /// Update the version number
    pub fn increment_version(&mut self) {
        self.version += 1;
    }

    /// Check if this metadata is compatible with another version
    pub fn is_compatible_with(&self, other_version: u64) -> bool {
        // For now, simple exact version matching
        // Could be extended to support backwards compatibility
        self.version == other_version
    }

    /// Get age of the metadata in seconds
    pub fn age_seconds(&self) -> Option<u64> {
        self.created_at.elapsed().ok().map(|d| d.as_secs())
    }

    /// Validate the metadata
    pub fn validate(&self) -> crate::Result<()> {
        use crate::error::RenoirError;
        
        if self.name.is_empty() {
            return Err(RenoirError::invalid_parameter("name", "Region name cannot be empty"));
        }
        
        if self.size == 0 {
            return Err(RenoirError::invalid_parameter("size", "Region size must be greater than 0"));
        }
        
        if self.version == 0 {
            return Err(RenoirError::invalid_parameter("version", "Version must be greater than 0"));
        }
        
        Ok(())
    }
}

/// Registry entry for a shared memory region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionRegistryEntry {
    /// Region metadata
    pub metadata: RegionMetadata,
    /// Process ID that created the region
    pub creator_pid: u32,
    /// Number of active references
    pub ref_count: u32,
    /// Last access timestamp
    pub last_accessed: SystemTime,
}

impl RegionRegistryEntry {
    /// Create a new registry entry
    pub fn new(metadata: RegionMetadata) -> Self {
        Self {
            metadata,
            creator_pid: std::process::id(),
            ref_count: 1,
            last_accessed: SystemTime::now(),
        }
    }

    /// Increment reference count
    pub fn add_ref(&mut self) {
        self.ref_count += 1;
        self.last_accessed = SystemTime::now();
    }

    /// Decrement reference count and return true if entry should be removed
    pub fn remove_ref(&mut self) -> bool {
        if self.ref_count > 0 {
            self.ref_count -= 1;
        }
        self.ref_count == 0
    }

    /// Update last accessed time
    pub fn touch(&mut self) {
        self.last_accessed = SystemTime::now();
    }

    /// Check if the entry is stale (no recent access)
    pub fn is_stale(&self, threshold_seconds: u64) -> bool {
        self.last_accessed
            .elapsed()
            .map(|duration| duration.as_secs() > threshold_seconds)
            .unwrap_or(false)
    }

    /// Get the age of this entry in seconds
    pub fn age_seconds(&self) -> Option<u64> {
        self.last_accessed.elapsed().ok().map(|d| d.as_secs())
    }

    /// Check if the creator process is still alive (Unix-specific)
    #[cfg(unix)]
    pub fn creator_alive(&self) -> bool {
        use std::process::{Command, Stdio};
        
        Command::new("kill")
            .arg("-0")
            .arg(self.creator_pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    /// Check if the creator process is still alive (non-Unix fallback)
    #[cfg(not(unix))]
    pub fn creator_alive(&self) -> bool {
        // On non-Unix systems, we can't easily check if a PID is alive
        // Return true to be conservative
        true
    }
}

/// Statistics for the control region
#[derive(Debug, Clone)]
pub struct ControlStats {
    /// Total number of registered regions
    pub total_regions: usize,
    /// Total size of the control region
    pub total_size: u64,
    /// Current global sequence number
    pub global_sequence: u64,
    /// When the control region was created
    pub created_at: SystemTime,
    /// When the control region was last modified
    pub last_modified: SystemTime,
}

impl ControlStats {
    /// Create new statistics
    pub fn new(
        total_regions: usize,
        total_size: u64,
        global_sequence: u64,
        created_at: SystemTime,
        last_modified: SystemTime,
    ) -> Self {
        Self {
            total_regions,
            total_size,
            global_sequence,
            created_at,
            last_modified,
        }
    }

    /// Get age of the control region in seconds
    pub fn age_seconds(&self) -> Option<u64> {
        self.created_at.elapsed().ok().map(|d| d.as_secs())
    }

    /// Get seconds since last modification
    pub fn idle_seconds(&self) -> Option<u64> {
        self.last_modified.elapsed().ok().map(|d| d.as_secs())
    }

    /// Check if the control region is active (recently modified)
    pub fn is_active(&self, threshold_seconds: u64) -> bool {
        self.idle_seconds()
            .map(|idle| idle <= threshold_seconds)
            .unwrap_or(true)
    }
}