//! Shared memory region management and operations

use std::{
    collections::HashMap,
    ffi::CString,
    fs::{File, OpenOptions},
    os::fd::{AsRawFd, RawFd, FromRawFd},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use memmap2::{MmapMut, MmapOptions};
use serde::{Deserialize, Serialize};

use crate::{
    error::{RenoirError, Result},
    metadata::{ControlRegion, RegionMetadata},
};

/// Types of shared memory backing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackingType {
    /// File-backed shared memory
    FileBacked,
    /// Anonymous memory file descriptor (Linux-specific)
    #[cfg(target_os = "linux")]
    MemFd,
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
            backing_type: BackingType::FileBacked,
            file_path: None,
            create: true,
            permissions: 0o644,
        }
    }
}

/// A shared memory region with its associated metadata
#[derive(Debug)]
pub struct SharedMemoryRegion {
    /// Region metadata
    metadata: RegionMetadata,
    /// Memory-mapped region
    mmap: MmapMut,
    /// Optional file handle for file-backed regions
    _file: Option<File>,
    /// Raw file descriptor
    fd: RawFd,
}

impl SharedMemoryRegion {
    /// Create or open a shared memory region
    pub fn new(config: RegionConfig) -> Result<Self> {
        if config.name.is_empty() {
            return Err(RenoirError::invalid_parameter("name", "Region name cannot be empty"));
        }

        if config.size == 0 {
            return Err(RenoirError::invalid_parameter("size", "Region size must be greater than 0"));
        }

        let (file, fd) = match config.backing_type {
            BackingType::FileBacked => {
                let path = config.file_path
                    .unwrap_or_else(|| PathBuf::from(format!("/tmp/renoir_{}", config.name)));
                
                let file = if config.create {
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .mode(config.permissions)
                        .open(&path)
                        .map_err(|e| RenoirError::from_io(e, "Failed to create/open file"))?
                } else {
                    File::open(&path)
                        .map_err(|e| RenoirError::from_io(e, "Failed to open existing file"))?
                };

                // Set file size if creating
                if config.create {
                    file.set_len(config.size as u64)
                        .map_err(|e| RenoirError::from_io(e, "Failed to set file size"))?;
                }

                let fd = file.as_raw_fd();
                (Some(file), fd)
            }
            #[cfg(target_os = "linux")]
            BackingType::MemFd => {
                let name_cstr = CString::new(config.name.clone())
                    .map_err(|_| RenoirError::invalid_parameter("name", "Name contains null bytes"))?;
                
                let fd = unsafe {
                    libc::memfd_create(name_cstr.as_ptr(), libc::MFD_CLOEXEC)
                };

                if fd == -1 {
                    return Err(RenoirError::platform("Failed to create memfd"));
                }

                // Set size using ftruncate
                if unsafe { libc::ftruncate(fd, config.size as i64) } == -1 {
                    unsafe { libc::close(fd); }
                    return Err(RenoirError::platform("Failed to set memfd size"));
                }

                (None, fd)
            }
        };

        // Create memory mapping
        let mmap = match &file {
            Some(f) => {
                // For file-backed regions, use the File reference
                unsafe {
                    MmapOptions::new()
                        .len(config.size)
                        .map_mut(f)
                        .map_err(|e| RenoirError::from_io(e, "Failed to create memory mapping"))?
                }
            }
            None => {
                // For memfd regions, create a temporary File from fd but don't let it drop
                let temp_file = unsafe { File::from_raw_fd(fd) };
                let mmap = unsafe {
                    MmapOptions::new()
                        .len(config.size)
                        .map_mut(&temp_file)
                        .map_err(|e| RenoirError::from_io(e, "Failed to create memory mapping"))?
                };
                // Don't let temp_file drop and close the fd
                std::mem::forget(temp_file);
                mmap
            }
        };

        let metadata = RegionMetadata {
            name: config.name,
            size: config.size,
            backing_type: config.backing_type,
            created_at: std::time::SystemTime::now(),
            version: 1,
        };

        Ok(Self {
            metadata,
            mmap,
            _file: file,
            fd,
        })
    }

    /// Get the region metadata
    pub fn metadata(&self) -> &RegionMetadata {
        &self.metadata
    }

    /// Get the raw memory slice (read-only)
    pub fn as_slice(&self) -> &[u8] {
        &self.mmap
    }

    /// Get the raw memory slice (mutable)
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.mmap
    }

    /// Get a typed pointer to the start of the region
    pub fn as_ptr<T>(&self) -> *const T {
        self.mmap.as_ptr() as *const T
    }

    /// Get a mutable typed pointer to the start of the region
    pub fn as_mut_ptr<T>(&mut self) -> *mut T {
        self.mmap.as_mut_ptr() as *mut T
    }

    /// Get a mutable typed pointer safely (for use in Arc contexts)
    /// 
    /// # Safety
    /// Caller must ensure exclusive access to the memory region
    pub unsafe fn as_mut_ptr_unsafe<T>(&self) -> *mut T {
        self.mmap.as_ptr() as *mut T
    }

    /// Get the size of the region
    pub fn size(&self) -> usize {
        self.metadata.size
    }

    /// Get the name of the region
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// Flush changes to persistent storage (for file-backed regions)
    pub fn flush(&self) -> Result<()> {
        self.mmap.flush()
            .map_err(|e| RenoirError::from_io(e, "Failed to flush memory mapping"))
    }

    /// Flush changes asynchronously
    pub fn flush_async(&self) -> Result<()> {
        self.mmap.flush_async()
            .map_err(|e| RenoirError::from_io(e, "Failed to flush memory mapping asynchronously"))
    }

    /// Get the file descriptor
    pub fn fd(&self) -> RawFd {
        self.fd
    }
}

impl Drop for SharedMemoryRegion {
    fn drop(&mut self) {
        // For memfd regions, we need to manually close the file descriptor
        // since we used std::mem::forget() on the temporary File
        if self._file.is_none() && self.fd != -1 {
            #[cfg(target_os = "linux")]
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}

unsafe impl Send for SharedMemoryRegion {}
unsafe impl Sync for SharedMemoryRegion {}

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
            control.register_region(&region.metadata)?;
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
}

impl Default for SharedMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_region_config_default() {
        let config = RegionConfig::default();
        assert_eq!(config.backing_type, BackingType::FileBacked);
        assert!(config.create);
        assert_eq!(config.permissions, 0o644);
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
    }

    #[test]
    fn test_shared_memory_manager() {
        let manager = SharedMemoryManager::new();
        assert_eq!(manager.region_count(), 0);

        let temp_dir = TempDir::new().unwrap();
        let config = RegionConfig {
            name: "test_region".to_string(),
            size: 4096,
            backing_type: BackingType::FileBacked,
            file_path: Some(temp_dir.path().join("test_shm")),
            create: true,
            permissions: 0o644,
        };

        let _region = manager.create_region(config).unwrap();
        assert_eq!(manager.region_count(), 1);
        
        let retrieved = manager.get_region("test_region").unwrap();
        assert_eq!(retrieved.name(), "test_region");

        manager.remove_region("test_region").unwrap();
        assert_eq!(manager.region_count(), 0);
    }
}