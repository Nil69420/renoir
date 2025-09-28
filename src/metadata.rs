//! Metadata management and control regions

use std::{
    collections::HashMap,
    sync::{atomic::AtomicU64, RwLock, Arc},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::{
    error::{RenoirError, Result},
    memory::{BackingType, RegionConfig, SharedMemoryRegion},
};

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

    /// Decrement reference count
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
}

/// Header for the control region containing global metadata
#[repr(C)]
pub struct ControlHeader {
    /// Magic number for validation
    pub magic: u64,
    /// Version of the control structure
    pub version: u64,
    /// Total size of the control region
    pub total_size: u64,
    /// Offset to the registry section
    pub registry_offset: u64,
    /// Size of the registry section
    pub registry_size: u64,
    /// Number of registered regions
    pub region_count: u64,
    /// Global sequence number
    pub global_sequence: AtomicU64,
    /// Creation timestamp
    pub created_at: u64,
    /// Last modified timestamp
    pub last_modified: AtomicU64,
}

impl Default for ControlHeader {
    fn default() -> Self {
        Self {
            magic: 0x52454E4F49520001, // "RENOIR" + version
            version: 1,
            total_size: 0,
            registry_offset: 0,
            registry_size: 0,
            region_count: 0,
            global_sequence: AtomicU64::new(0),
            created_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            last_modified: AtomicU64::new(0),
        }
    }
}

/// Control region for managing shared memory regions and metadata
#[derive(Debug)]
pub struct ControlRegion {
    /// The shared memory region backing this control region
    region: Arc<SharedMemoryRegion>,
    /// Registry of managed regions
    registry: RwLock<HashMap<String, RegionRegistryEntry>>,
    /// Header reference
    header: *mut ControlHeader,
}

impl ControlRegion {
    /// Create or open a control region
    pub fn new(mut config: RegionConfig) -> Result<Self> {
        // Ensure minimum size for control region
        if config.size < std::mem::size_of::<ControlHeader>() + 1024 {
            config.size = std::mem::size_of::<ControlHeader>() + 4096;
        }

        let region = Arc::new(SharedMemoryRegion::new(config)?);
        
        // Get pointer to header
        let header = region.as_ptr::<ControlHeader>() as *mut ControlHeader;
        
        // Initialize or validate header
        let is_new = unsafe {
            let current_magic = (*header).magic;
            if current_magic == 0 || current_magic != ControlHeader::default().magic {
                // Initialize new header
                *header = ControlHeader::default();
                (*header).total_size = region.size() as u64;
                (*header).registry_offset = std::mem::size_of::<ControlHeader>() as u64;
                (*header).registry_size = (region.size() - std::mem::size_of::<ControlHeader>()) as u64;
                true
            } else {
                false
            }
        };

        let registry = if is_new {
            HashMap::new()
        } else {
            // Load existing registry from shared memory
            Self::load_registry_from_memory(&region)?
        };

        Ok(Self {
            region,
            registry: RwLock::new(registry),
            header,
        })
    }

    /// Load registry from shared memory
    fn load_registry_from_memory(region: &SharedMemoryRegion) -> Result<HashMap<String, RegionRegistryEntry>> {
        let header = unsafe { &*(region.as_ptr::<ControlHeader>()) };
        
        if header.registry_size == 0 || header.region_count == 0 {
            return Ok(HashMap::new());
        }

        let registry_start = header.registry_offset as usize;
        let registry_slice = &region.as_slice()[registry_start..];
        
        // Deserialize registry
        bincode::deserialize(registry_slice)
            .map_err(|e| RenoirError::serialization(format!("Failed to load registry: {}", e)))
    }

    /// Save registry to shared memory
    fn save_registry_to_memory(&self) -> Result<()> {
        let registry = self.registry.read().unwrap();
        let serialized = bincode::serialize(&*registry)
            .map_err(|e| RenoirError::serialization(format!("Failed to serialize registry: {}", e)))?;

        let header = unsafe { &mut *self.header };
        let registry_start = header.registry_offset as usize;
        let available_size = header.registry_size as usize;

        if serialized.len() > available_size {
            return Err(RenoirError::insufficient_space(serialized.len(), available_size));
        }

        // Copy serialized data to shared memory
        let region_slice = unsafe {
            std::slice::from_raw_parts_mut(
                self.region.as_ptr::<u8>().add(registry_start) as *mut u8,
                available_size,
            )
        };

        region_slice[..serialized.len()].copy_from_slice(&serialized);
        
        // Update header
        header.region_count = registry.len() as u64;
        header.last_modified.store(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            std::sync::atomic::Ordering::SeqCst,
        );

        // Flush changes
        self.region.flush()?;
        
        Ok(())
    }

    /// Register a new region
    pub fn register_region(&self, metadata: &RegionMetadata) -> Result<()> {
        {
            let mut registry = self.registry.write().unwrap();
            if registry.contains_key(&metadata.name) {
                return Err(RenoirError::region_exists(&metadata.name));
            }

            let entry = RegionRegistryEntry::new(metadata.clone());
            registry.insert(metadata.name.clone(), entry);
        }

        self.save_registry_to_memory()?;
        self.increment_sequence();
        
        Ok(())
    }

    /// Unregister a region
    pub fn unregister_region(&self, name: &str) -> Result<()> {
        {
            let mut registry = self.registry.write().unwrap();
            registry.remove(name)
                .ok_or_else(|| RenoirError::region_not_found(name))?;
        }

        self.save_registry_to_memory()?;
        self.increment_sequence();
        
        Ok(())
    }

    /// Get region registry entry
    pub fn get_region_entry(&self, name: &str) -> Option<RegionRegistryEntry> {
        let registry = self.registry.read().unwrap();
        registry.get(name).cloned()
    }

    /// List all registered regions
    pub fn list_regions(&self) -> Vec<RegionRegistryEntry> {
        let registry = self.registry.read().unwrap();
        registry.values().cloned().collect()
    }

    /// Add reference to a region
    pub fn add_region_ref(&self, name: &str) -> Result<()> {
        {
            let mut registry = self.registry.write().unwrap();
            let entry = registry.get_mut(name)
                .ok_or_else(|| RenoirError::region_not_found(name))?;
            entry.add_ref();
        }

        self.save_registry_to_memory()?;
        
        Ok(())
    }

    /// Remove reference from a region
    pub fn remove_region_ref(&self, name: &str) -> Result<bool> {
        let should_remove = {
            let mut registry = self.registry.write().unwrap();
            let entry = registry.get_mut(name)
                .ok_or_else(|| RenoirError::region_not_found(name))?;
            entry.remove_ref()
        };

        if should_remove {
            self.unregister_region(name)?;
        } else {
            self.save_registry_to_memory()?;
        }

        Ok(should_remove)
    }

    /// Get current global sequence number
    pub fn get_sequence(&self) -> u64 {
        let header = unsafe { &*self.header };
        header.global_sequence.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Increment global sequence number
    pub fn increment_sequence(&self) -> u64 {
        let header = unsafe { &*self.header };
        header.global_sequence.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    /// Get control region statistics
    pub fn get_stats(&self) -> ControlStats {
        let header = unsafe { &*self.header };
        let registry = self.registry.read().unwrap();

        ControlStats {
            total_regions: registry.len(),
            total_size: header.total_size,
            global_sequence: self.get_sequence(),
            created_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(header.created_at),
            last_modified: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(
                header.last_modified.load(std::sync::atomic::Ordering::SeqCst)
            ),
        }
    }

    /// Get the underlying shared memory region
    pub fn region(&self) -> &Arc<SharedMemoryRegion> {
        &self.region
    }
}

unsafe impl Send for ControlRegion {}
unsafe impl Sync for ControlRegion {}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::BackingType;
    use tempfile::TempDir;

    #[test]
    fn test_region_metadata() {
        let metadata = RegionMetadata::new(
            "test".to_string(),
            4096,
            BackingType::FileBacked,
        );
        
        assert_eq!(metadata.name, "test");
        assert_eq!(metadata.size, 4096);
        assert_eq!(metadata.version, 1);
    }

    #[test]
    fn test_registry_entry() {
        let metadata = RegionMetadata::new(
            "test".to_string(),
            4096,
            BackingType::FileBacked,
        );
        
        let mut entry = RegionRegistryEntry::new(metadata);
        assert_eq!(entry.ref_count, 1);
        
        entry.add_ref();
        assert_eq!(entry.ref_count, 2);
        
        assert!(!entry.remove_ref());
        assert_eq!(entry.ref_count, 1);
        
        assert!(entry.remove_ref());
        assert_eq!(entry.ref_count, 0);
    }

    #[test]
    fn test_control_region() {
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
        
        let metadata = RegionMetadata::new(
            "test_region".to_string(),
            4096,
            BackingType::FileBacked,
        );
        
        control.register_region(&metadata).unwrap();
        
        let entry = control.get_region_entry("test_region").unwrap();
        assert_eq!(entry.metadata.name, "test_region");
        
        let stats = control.get_stats();
        assert_eq!(stats.total_regions, 1);
    }
}