use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nix::fcntl::{flock, FlockArg};

use crate::error::{RenoirError, Result};
use crate::memory::config::region_path;

use super::response_ring_name;

const MIN_AGE: Duration = Duration::from_secs(5);
const ID_HEX_LEN: usize = 8;
const LEASE_SUFFIX: &str = ".lease";

fn lease_path(ring: &Path) -> PathBuf {
    let mut name = ring.as_os_str().to_owned();
    name.push(LEASE_SUFFIX);
    PathBuf::from(name)
}

pub(super) struct ResponseLease {
    _file: File,
    ring: PathBuf,
    lease: PathBuf,
}

impl ResponseLease {
    pub(super) fn acquire(ring_name: &str) -> Result<Self> {
        let ring = region_path(ring_name);
        let lease = lease_path(&ring);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lease)
            .map_err(|e| RenoirError::from_io(e, "Failed to create the response lease"))?;
        flock(file.as_raw_fd(), FlockArg::LockSharedNonblock)
            .map_err(|e| RenoirError::platform(format!("flock failed: {e}")))?;
        Ok(Self {
            _file: file,
            ring,
            lease,
        })
    }
}

impl Drop for ResponseLease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.ring);
        let _ = std::fs::remove_file(&self.lease);
    }
}

pub(super) fn reap_abandoned(service: &str) -> usize {
    let probe = region_path(&response_ring_name(service, 0));
    let (Some(dir), Some(file_name)) = (probe.parent(), probe.file_name()) else {
        return 0;
    };
    let file_name = file_name.to_string_lossy();
    let prefix = &file_name[..file_name.len() - ID_HEX_LEN];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let id_ok = |s: &str| s.len() == ID_HEX_LEN && s.bytes().all(|b| b.is_ascii_hexdigit());
        let ring = if id_ok(rest) {
            entry.path()
        } else if rest.strip_suffix(LEASE_SUFFIX).is_some_and(id_ok) {
            entry.path().with_extension("")
        } else {
            continue;
        };
        if reap_one(&ring) {
            removed += 1;
        }
    }
    removed
}

fn reap_one(ring: &Path) -> bool {
    let lease = lease_path(ring);
    let old_enough = [ring, lease.as_path()]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .filter_map(|m| m.modified().ok())
        .filter_map(|t| t.elapsed().ok())
        .min()
        .is_some_and(|age| age >= MIN_AGE);
    if !old_enough {
        return false;
    }
    if lease.exists() {
        let Ok(file) = OpenOptions::new().read(true).write(true).open(&lease) else {
            return false;
        };
        if flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock).is_err() {
            return false;
        }
    }
    let ring_gone = std::fs::remove_file(ring).is_ok() || !ring.exists();
    let lease_gone = std::fs::remove_file(&lease).is_ok() || !lease.exists();
    ring_gone && lease_gone
}
