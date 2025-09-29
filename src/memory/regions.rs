//! Shared memory region implementation

use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    os::fd::{AsRawFd, RawFd, OwnedFd},
    os::unix::fs::OpenOptionsExt,
};

use memmap2::{MmapMut, MmapOptions};
use nix::{
    sys::memfd::{memfd_create, MemFdCreateFlag},
    unistd::ftruncate,
};

use crate::{
    error::{RenoirError, Result},
    metadata_modules::RegionMetadata,
};

use super::config::{BackingType, RegionConfig};

/// A shared memory region with its associated metadata
#[derive(Debug)]
pub struct SharedMemoryRegion {
    /// Region metadata
    metadata: RegionMetadata,
    /// Memory-mapped region
    mmap: MmapMut,
    /// Optional file handle for file-backed regions
    _file: Option<File>,
    /// Owned file descriptor for memfd regions
    _owned_fd: Option<OwnedFd>,
    /// Raw file descriptor (for compatibility with existing APIs)
    fd: RawFd,
}

impl SharedMemoryRegion {
    /// Create or open a shared memory region
    pub fn new(config: RegionConfig) -> Result<Self> {
        // Validate configuration
        config.validate()?;

        let (file, owned_fd, fd) = Self::create_backing(&config)?;
        let mmap = Self::create_mapping(&file, &owned_fd, fd, config.size)?;

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
            _owned_fd: owned_fd,
            fd,
        })
    }

    /// Create the backing storage for the region
    fn create_backing(config: &RegionConfig) -> Result<(Option<File>, Option<OwnedFd>, RawFd)> {
        match config.backing_type {
            BackingType::FileBacked => {
                Self::create_file_backing(config)
            }
            #[cfg(target_os = "linux")]
            BackingType::MemFd => {
                Self::create_memfd_backing(config)
            }
        }
    }

    /// Create file-backed storage
    fn create_file_backing(config: &RegionConfig) -> Result<(Option<File>, Option<OwnedFd>, RawFd)> {
        let path = config.default_file_path();
        
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
        Ok((Some(file), None, fd))
    }

    /// Create memfd-backed storage
    #[cfg(target_os = "linux")]
    fn create_memfd_backing(config: &RegionConfig) -> Result<(Option<File>, Option<OwnedFd>, RawFd)> {
        let name_cstr = CString::new(config.name.clone())
            .map_err(|_| RenoirError::invalid_parameter("name", "Name contains null bytes"))?;
        
        let owned_fd = memfd_create(&name_cstr, MemFdCreateFlag::MFD_CLOEXEC)
            .map_err(|e| RenoirError::platform(&format!("Failed to create memfd: {}", e)))?;

        let raw_fd = owned_fd.as_raw_fd();

        // Set size using ftruncate
        ftruncate(&owned_fd, config.size as i64)
            .map_err(|e| {
                RenoirError::platform(&format!("Failed to set memfd size: {}", e))
            })?;

        Ok((None, Some(owned_fd), raw_fd))
    }

    /// Create memory mapping for the backing storage
    fn create_mapping(file: &Option<File>, owned_fd: &Option<OwnedFd>, _fd: RawFd, size: usize) -> Result<MmapMut> {
        match file {
            Some(f) => {
                // For file-backed regions, use the File reference
                unsafe {
                    MmapOptions::new()
                        .len(size)
                        .map_mut(f)
                        .map_err(|e| RenoirError::from_io(e, "Failed to create memory mapping"))
                }
            }
            None => {
                // For memfd regions, use the OwnedFd directly
                if let Some(owned_fd) = owned_fd {
                    unsafe {
                        MmapOptions::new()
                            .len(size)
                            .map_mut(owned_fd)
                            .map_err(|e| RenoirError::from_io(e, "Failed to create memory mapping"))
                    }
                } else {
                    return Err(RenoirError::platform("No file or owned fd available for mapping"));
                }
            }
        }
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

    /// Check if the region is file-backed
    pub fn is_file_backed(&self) -> bool {
        matches!(self.metadata.backing_type, BackingType::FileBacked)
    }

    /// Check if the region is memfd-backed
    #[cfg(target_os = "linux")]
    pub fn is_memfd_backed(&self) -> bool {
        matches!(self.metadata.backing_type, BackingType::MemFd)
    }

    /// Get memory statistics for this region
    pub fn memory_stats(&self) -> RegionMemoryStats {
        RegionMemoryStats {
            name: self.name().to_string(),
            size: self.size(),
            backing_type: self.metadata.backing_type,
            created_at: self.metadata.created_at,
            fd: self.fd,
        }
    }
}

impl Drop for SharedMemoryRegion {
    fn drop(&mut self) {
        // If we have an OwnedFd, it will automatically close when dropped
        // Only manually close if we have neither a File nor an OwnedFd (shouldn't happen in normal usage)
        if self._file.is_none() && self._owned_fd.is_none() && self.fd != -1 {
            #[cfg(target_os = "linux")]
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}

unsafe impl Send for SharedMemoryRegion {}
unsafe impl Sync for SharedMemoryRegion {}

/// Memory statistics for a region
#[derive(Debug, Clone)]
pub struct RegionMemoryStats {
    pub name: String,
    pub size: usize,
    pub backing_type: BackingType,
    pub created_at: std::time::SystemTime,
    pub fd: RawFd,
}

impl RegionMemoryStats {
    /// Get the age of the region in seconds
    pub fn age_seconds(&self) -> Option<u64> {
        self.created_at.elapsed().ok().map(|d| d.as_secs())
    }
}