//! Control region for managing shared memory regions and metadata

use std::{
    collections::HashMap,
    sync::{atomic::AtomicU64, RwLock, Arc},
    time::SystemTime,
};

use crate::{
    error::{RenoirError, Result},
    memory::{RegionConfig, SharedMemoryRegion},
};

use super::types::{RegionMetadata, RegionRegistryEntry, ControlStats};

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

impl ControlHeader {
    /// Magic number constant
    pub const MAGIC: u64 = 0x52454E4F49520001;
    
    /// Current version constant
    pub const VERSION: u64 = 1;
    
    /// Validate the header
    pub fn validate(&self) -> Result<()> {
        if self.magic != Self::MAGIC {
            return Err(RenoirError::invalid_parameter("magic_number", "Invalid control header magic number"));
        }
        
        if self.version != Self::VERSION {
            return Err(RenoirError::invalid_parameter("version", &format!(
                "Unsupported control header version: {}",
                self.version
            )));
        }
        
        if self.total_size < std::mem::size_of::<ControlHeader>() as u64 {
            return Err(RenoirError::invalid_parameter("total_size", "Control header size too small"));
        }
        
        Ok(())
    }
    
    /// Initialize header with size
    pub fn initialize(&mut self, total_size: usize) {
        *self = Self::default();
        self.total_size = total_size as u64;
        self.registry_offset = std::mem::size_of::<ControlHeader>() as u64;
        self.registry_size = (total_size - std::mem::size_of::<ControlHeader>()) as u64;
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
        let min_size = std::mem::size_of::<ControlHeader>() + 1024;
        if config.size < min_size {
            config.size = std::mem::size_of::<ControlHeader>() + 4096;
        }

        let region = Arc::new(SharedMemoryRegion::new(config)?);
        
        // Get pointer to header
        let header = region.as_ptr::<ControlHeader>() as *mut ControlHeader;
        
        // Initialize or validate header
        let is_new = unsafe {
            let current_magic = (*header).magic;
            if current_magic == 0 || current_magic != ControlHeader::MAGIC {
                // Initialize new header
                (*header).initialize(region.size());
                true
            } else {
                // Validate existing header
                (*header).validate()?;
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
        let registry_end = registry_start + header.registry_size as usize;
        
        if registry_end > region.size() {
            return Err(RenoirError::invalid_parameter("registry_bounds", "Registry extends beyond region bounds"));
        }
        
        let registry_slice = &region.as_slice()[registry_start..registry_end];
        
        // Find the actual used portion (bincode might not use the full space)
        if registry_slice.is_empty() || registry_slice[0] == 0 {
            return Ok(HashMap::new());
        }
        
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

        // Clear the region first
        region_slice.fill(0);
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
        // Validate metadata first
        metadata.validate()?;
        
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

        ControlStats::new(
            registry.len(),
            header.total_size,
            self.get_sequence(),
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(header.created_at),
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(
                header.last_modified.load(std::sync::atomic::Ordering::SeqCst)
            ),
        )
    }

    /// Get the underlying shared memory region
    pub fn region(&self) -> &Arc<SharedMemoryRegion> {
        &self.region
    }

    /// Clean up stale entries (entries from dead processes)
    pub fn cleanup_stale_entries(&self, max_age_seconds: u64) -> Result<usize> {
        let stale_names: Vec<String> = {
            let registry = self.registry.read().unwrap();
            registry.iter()
                .filter(|(_, entry)| {
                    entry.is_stale(max_age_seconds) && !entry.creator_alive()
                })
                .map(|(name, _)| name.clone())
                .collect()
        };

        let removed_count = stale_names.len();
        
        for name in stale_names {
            let _ = self.unregister_region(&name); // Ignore errors for cleanup
        }

        Ok(removed_count)
    }
}

unsafe impl Send for ControlRegion {}
unsafe impl Sync for ControlRegion {}