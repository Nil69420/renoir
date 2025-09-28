//! Shared memory manager for multiple regions

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::{
    error::{RenoirError, Result},
    metadata_modules::ControlRegion,
};

use super::{
    config::RegionConfig,
    regions::{SharedMemoryRegion, RegionMemoryStats},
};

/// Manager for multiple shared memory regions
#[derive(Debug)]
pub struct SharedMemoryManager {
    /// Map of region names to regions
    regions: RwLock<HashMap<String, Arc<SharedMemoryRegion>>>,
    /// Control region for metadata
    control_region: Option<Arc<ControlRegion>>,
}

impl SharedMemoryManager {
    /// Create a new shared memory manager
    pub fn new() -> Self {
        Self {
            regions: RwLock::new(HashMap::new()),
            control_region: None,
        }
    }

    /// Create a new shared memory manager with a control region
    pub fn with_control_region(control_config: RegionConfig) -> Result<Self> {
        let control_region = ControlRegion::new(control_config)?;
        
        Ok(Self {
            regions: RwLock::new(HashMap::new()),
            control_region: Some(Arc::new(control_region)),
        })
    }

    /// Create or open a shared memory region
    pub fn create_region(&self, config: RegionConfig) -> Result<Arc<SharedMemoryRegion>> {
        let region_name = config.name.clone();
        
        {
            let regions = self.regions.read().unwrap();
            if regions.contains_key(&region_name) {
                return Err(RenoirError::region_exists(&region_name));
            }
        }

        let region = Arc::new(SharedMemoryRegion::new(config)?);

        {
            let mut regions = self.regions.write().unwrap();
            regions.insert(region_name.clone(), Arc::clone(&region));
        }

        // Register with control region if available
        if let Some(control) = &self.control_region {
            control.register_region(&region.metadata())?;
        }

        Ok(region)
    }

    /// Get an existing shared memory region
    pub fn get_region(&self, name: &str) -> Result<Arc<SharedMemoryRegion>> {
        let regions = self.regions.read().unwrap();
        regions.get(name)
            .cloned()
            .ok_or_else(|| RenoirError::region_not_found(name))
    }

    /// Remove a shared memory region
    pub fn remove_region(&self, name: &str) -> Result<()> {
        {
            let mut regions = self.regions.write().unwrap();
            regions.remove(name)
                .ok_or_else(|| RenoirError::region_not_found(name))?;
        }

        // Unregister from control region if available
        if let Some(control) = &self.control_region {
            control.unregister_region(name)?;
        }

        Ok(())
    }

    /// List all managed regions
    pub fn list_regions(&self) -> Vec<String> {
        let regions = self.regions.read().unwrap();
        regions.keys().cloned().collect()
    }

    /// Get the number of managed regions
    pub fn region_count(&self) -> usize {
        let regions = self.regions.read().unwrap();
        regions.len()
    }

    /// Get access to the control region
    pub fn control_region(&self) -> Option<Arc<ControlRegion>> {
        self.control_region.clone()
    }

    /// Get memory statistics for all regions
    pub fn memory_stats(&self) -> Vec<RegionMemoryStats> {
        let regions = self.regions.read().unwrap();
        regions.values()
            .map(|region| region.memory_stats())
            .collect()
    }

    /// Get total memory usage across all regions
    pub fn total_memory_usage(&self) -> usize {
        let regions = self.regions.read().unwrap();
        regions.values()
            .map(|region| region.size())
            .sum()
    }

    /// Check if a region exists
    pub fn has_region(&self, name: &str) -> bool {
        let regions = self.regions.read().unwrap();
        regions.contains_key(name)
    }

    /// Get region by name (returns None if not found)
    pub fn try_get_region(&self, name: &str) -> Option<Arc<SharedMemoryRegion>> {
        let regions = self.regions.read().unwrap();
        regions.get(name).cloned()
    }

    /// Remove all regions
    pub fn clear(&self) -> Result<()> {
        let region_names: Vec<String> = {
            let regions = self.regions.read().unwrap();
            regions.keys().cloned().collect()
        };

        for name in region_names {
            self.remove_region(&name)?;
        }

        Ok(())
    }

    /// Flush all regions to persistent storage
    pub fn flush_all(&self) -> Result<()> {
        let regions = self.regions.read().unwrap();
        
        for region in regions.values() {
            region.flush()?;
        }
        
        Ok(())
    }

    /// Get regions filtered by backing type
    pub fn regions_by_backing_type(&self, backing_type: super::config::BackingType) -> Vec<Arc<SharedMemoryRegion>> {
        let regions = self.regions.read().unwrap();
        regions.values()
            .filter(|region| region.metadata().backing_type == backing_type)
            .cloned()
            .collect()
    }
}

impl Default for SharedMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}