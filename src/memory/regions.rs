//! Shared memory region implementation

use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    os::fd::{AsRawFd, OwnedFd, RawFd},
    os::unix::fs::{FileExt, OpenOptionsExt},
};

use memmap2::{MmapMut, MmapOptions};
use nix::{
    sys::memfd::{memfd_create, MemFdCreateFlag},
    unistd::ftruncate,
};

use crate::{
    error::{RenoirError, Result},
    metadata_modules::RegionMetadata,
    topic_rings::{
        BROADCAST_RING_MAGIC, MPMC_RING_MAGIC, SESSION_CLEAN, SESSION_DIRTY, SESSION_MARKER_OFFSET,
        SPSC_RING_MAGIC,
    },
};

use super::config::{BackingType, RegionConfig};

/// Session-liveness state of a file-backed region, determined at open time
#[derive(Debug, Clone, Copy)]
struct SessionInfo {
    /// Whether this process acquired the exclusive lock (session owner)
    owner: bool,
}

impl SessionInfo {
    fn non_owner() -> Self {
        Self { owner: false }
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
    /// Owned file descriptor for memfd regions
    _owned_fd: Option<OwnedFd>,
    /// Raw file descriptor (for compatibility with existing APIs)
    fd: RawFd,
    /// Bytes currently allocated from this region by its pools
    allocated_bytes: std::sync::atomic::AtomicI64,
    /// Whether this process owns the session (holds the exclusive lock)
    session_owner: bool,
}

impl SharedMemoryRegion {
    /// Create or open a shared memory region
    pub fn new(config: RegionConfig) -> Result<Self> {
        // Validate configuration
        config.validate()?;

        let (file, owned_fd, fd, session) = Self::create_backing(&config)?;
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
            allocated_bytes: std::sync::atomic::AtomicI64::new(0),
            session_owner: session.owner,
        })
    }

    /// Mark this region's session as cleanly closed if this process owns it
    /// and the file now holds a ring. Called by Drop and by the FFI
    /// destroy/close paths — C and Python callers release regions via those
    /// paths, so relying only on Rust Drop would make every clean C/Python
    /// exit look identical to a crash.
    pub fn mark_session_clean(&self) {
        if !self.session_owner {
            return;
        }
        if let Some(file) = &self._file {
            let mut probe = [0u8; 32];
            let probe_len = file.read_at(&mut probe, 0).unwrap_or(0);
            if read_session_marker(&probe[..probe_len]).0 {
                let marker = [SESSION_CLEAN];
                let _ = file.write_at(&marker, SESSION_MARKER_OFFSET as u64);
            }
        }
    }

    /// Current number of bytes allocated from this region by its pools
    pub fn allocated_bytes(&self) -> usize {
        self.allocated_bytes
            .load(std::sync::atomic::Ordering::Relaxed)
            .max(0) as usize
    }

    /// Adjust the region's allocated-byte counter (used by pools)
    pub(crate) fn adjust_allocated(&self, delta: i64) {
        self.allocated_bytes
            .fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
    }

    /// Create the backing storage for the region
    fn create_backing(
        config: &RegionConfig,
    ) -> Result<(Option<File>, Option<OwnedFd>, RawFd, SessionInfo)> {
        match config.backing_type {
            BackingType::FileBacked => Self::create_file_backing(config),
            #[cfg(target_os = "linux")]
            BackingType::MemFd => Self::create_memfd_backing(config),
        }
    }

    /// Create file-backed storage
    fn create_file_backing(
        config: &RegionConfig,
    ) -> Result<(Option<File>, Option<OwnedFd>, RawFd, SessionInfo)> {
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

        // Only the session owner sizes the file; participants never resize a live region.
        let session = if config.create {
            // Liveness protocol: advisory flock() locks are tied to the
            // open-file-description and are released by the kernel the moment
            // every fd referencing them closes — including on crash/kill -9.
            // Try an exclusive lock first: success means no live process
            // holds this file (brand new, or orphaned from a dead process),
            // so it is safe to reinitialize the content. EWOULDBLOCK means a
            // live session holds a shared lock — attach without touching it.
            let fd = file.as_raw_fd();
            let mut session = SessionInfo::non_owner();
            match nix::fcntl::flock(fd, nix::fcntl::FlockArg::LockExclusiveNonblock) {
                Ok(()) => {
                    // No live holder. Read the session marker (if the file
                    // already holds a ring) to decide wipe-vs-preserve:
                    // DIRTY means the previous holder crashed mid-write,
                    // CLEAN means it exited cleanly and its messages are
                    // legitimate. A brand-new file has no ring magic.
                    let mut probe = [0u8; 32];
                    let probe_len = file.read_at(&mut probe, 0).unwrap_or(0);
                    let (is_ring, marker) = read_session_marker(&probe[..probe_len]);

                    if is_ring && marker == SESSION_CLEAN {
                        // Previous holder exited cleanly: preserve content
                        let current = file
                            .metadata()
                            .map_err(|e| RenoirError::from_io(e, "Failed to stat file"))?
                            .len();
                        if current < config.size as u64 {
                            file.set_len(config.size as u64)
                                .map_err(|e| RenoirError::from_io(e, "Failed to set file size"))?;
                        }
                    } else {
                        // Fresh file, non-ring region, or crashed session:
                        // zero the content so a fresh attach reinitializes
                        // instead of seeing the dead session's data
                        file.set_len(0)
                            .map_err(|e| RenoirError::from_io(e, "Failed to truncate file"))?;
                        file.set_len(config.size as u64)
                            .map_err(|e| RenoirError::from_io(e, "Failed to set file size"))?;
                    }

                    preallocate(fd, config.size)?;

                    // We are the new session owner. Mark DIRTY so a crash
                    // before our own clean close is detected by the next
                    // opener. (from_memory later re-initializes the header
                    // with the same value.)
                    if is_ring {
                        let marker = [SESSION_DIRTY];
                        let _ = file.write_at(&marker, SESSION_MARKER_OFFSET as u64);
                    }

                    session = SessionInfo { owner: true };
                }
                Err(nix::errno::Errno::EWOULDBLOCK) => {
                    // A live process holds the region; keep its content.
                    // We are a participant, never the session owner.
                    wait_for_size(&file, config.size, std::time::Duration::from_millis(500))?;
                }
                Err(e) => {
                    return Err(RenoirError::platform(format!("flock failed: {}", e)));
                }
            }
            session
        } else {
            wait_for_size(&file, config.size, std::time::Duration::ZERO)?;
            SessionInfo::non_owner()
        };

        // Hold a shared lock for this region's lifetime. Multiple processes
        // on the same region stack shared locks (MPMC fan-out is expected);
        // a shared lock blocks any future exclusive attempt, which is the
        // "someone's still alive" signal. If this call follows the exclusive
        // lock above on the same fd, flock converts the lock to shared mode.
        let fd = file.as_raw_fd();
        nix::fcntl::flock(fd, nix::fcntl::FlockArg::LockShared)
            .map_err(|e| RenoirError::platform(format!("flock shared failed: {}", e)))?;

        Ok((Some(file), None, fd, session))
    }

    /// Create memfd-backed storage
    #[cfg(target_os = "linux")]
    fn create_memfd_backing(
        config: &RegionConfig,
    ) -> Result<(Option<File>, Option<OwnedFd>, RawFd, SessionInfo)> {
        let name_cstr = CString::new(config.name.clone())
            .map_err(|_| RenoirError::invalid_parameter("name", "Name contains null bytes"))?;

        let owned_fd = memfd_create(&name_cstr, MemFdCreateFlag::MFD_CLOEXEC)
            .map_err(|e| RenoirError::platform(format!("Failed to create memfd: {}", e)))?;

        let raw_fd = owned_fd.as_raw_fd();

        // Set size using ftruncate
        ftruncate(&owned_fd, config.size as i64)
            .map_err(|e| RenoirError::platform(format!("Failed to set memfd size: {}", e)))?;
        preallocate(raw_fd, config.size)?;

        Ok((None, Some(owned_fd), raw_fd, SessionInfo::non_owner()))
    }

    /// Create memory mapping for the backing storage
    fn create_mapping(
        file: &Option<File>,
        owned_fd: &Option<OwnedFd>,
        _fd: RawFd,
        size: usize,
    ) -> Result<MmapMut> {
        match file {
            Some(f) => {
                // SAFETY: File is open and sized correctly; mapping is valid for the requested length.
                unsafe {
                    MmapOptions::new()
                        .len(size)
                        .map_mut(f)
                        .map_err(|e| RenoirError::from_io(e, "Failed to create memory mapping"))
                        .map(prefault_for_writing)
                }
            }
            None => {
                // For memfd regions, use the OwnedFd directly
                if let Some(owned_fd) = owned_fd {
                    // SAFETY: OwnedFd is valid and memfd has been sized via ftruncate.
                    unsafe {
                        MmapOptions::new()
                            .len(size)
                            .map_mut(owned_fd)
                            .map_err(|e| RenoirError::from_io(e, "Failed to create memory mapping"))
                            .map(prefault_for_writing)
                    }
                } else {
                    Err(RenoirError::platform(
                        "No file or owned fd available for mapping",
                    ))
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
        self.mmap
            .flush()
            .map_err(|e| RenoirError::from_io(e, "Failed to flush memory mapping"))
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
        // A graceful exit of the session owner is the only legitimate way
        // to leave the CLEAN marker: mark it before releasing the lock so
        // the next opener preserves this session's published messages.
        self.mark_session_clean();
        // If we have an OwnedFd, it will automatically close when dropped
        // Only manually close if we have neither a File nor an OwnedFd (shouldn't happen in normal usage)
        if self._file.is_none() && self._owned_fd.is_none() && self.fd != -1 {
            #[cfg(target_os = "linux")]
            // SAFETY: fd is valid and not owned by File or OwnedFd (checked above), so we must close it manually.
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}

/// Interpret the first bytes of a region file as a ring session marker.
/// Returns (is_ring, marker): is_ring is true only when the file already
/// holds a known ring control block, so non-ring regions (e.g. pool
/// regions) are never wiped by a marker decision and never get marker
/// bytes written into their data.
fn read_session_marker(probe: &[u8]) -> (bool, u8) {
    if probe.len() < SESSION_MARKER_OFFSET + 1 {
        return (false, SESSION_DIRTY);
    }
    let magic: [u8; 8] = probe[0..8].try_into().unwrap_or([0; 8]);
    if magic != SPSC_RING_MAGIC.to_le_bytes()
        && magic != MPMC_RING_MAGIC.to_le_bytes()
        && magic != BROADCAST_RING_MAGIC.to_le_bytes()
    {
        return (false, SESSION_DIRTY);
    }
    (true, probe[SESSION_MARKER_OFFSET])
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

fn preallocate(fd: RawFd, size: usize) -> Result<()> {
    match nix::fcntl::posix_fallocate(fd, 0, size as i64) {
        Ok(()) => Ok(()),
        Err(
            nix::errno::Errno::EOPNOTSUPP | nix::errno::Errno::ENOSYS | nix::errno::Errno::EINVAL,
        ) => Ok(()),
        Err(nix::errno::Errno::ENOSPC) => Err(RenoirError::platform(format!(
            "Not enough space for a {size} byte region in the shared-memory filesystem; \
             enlarge /dev/shm (for example docker run --shm-size) or reduce the ring size"
        ))),
        Err(e) => Err(RenoirError::platform(format!(
            "Failed to reserve {size} bytes for the region: {e}"
        ))),
    }
}

fn wait_for_size(file: &File, size: usize, grace: std::time::Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + grace;
    loop {
        let len = file
            .metadata()
            .map_err(|e| RenoirError::from_io(e, "Failed to stat file"))?
            .len();
        if len >= size as u64 {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(RenoirError::invalid_parameter(
                "size",
                "Existing region is smaller than the requested size (different geometry?)",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn prefault_for_writing(map: MmapMut) -> MmapMut {
    #[cfg(target_os = "linux")]
    {
        let _ = map.advise(memmap2::Advice::PopulateWrite);
    }
    map
}
