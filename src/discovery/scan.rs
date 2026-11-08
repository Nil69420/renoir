use std::{
    fs::{self, File},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

use super::procs::{mappings, Mappings};
use crate::memory::config::shm_dir;
use crate::topic_rings::{
    inspect_lean_header, LeanInfo, BROADCAST_RING_MAGIC, LEAN_HEADER_BYTES, MPMC_RING_MAGIC,
    SPSC_RING_MAGIC,
};

pub(super) const FILE_PREFIX: &str = "renoir_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingKind {
    Lean,
    Broadcast,
    Spsc,
    Mpmc,
}

#[derive(Debug, Clone)]
pub struct RingInfo {
    pub name: String,
    pub path: PathBuf,
    pub kind: RingKind,
    pub file_bytes: u64,
    pub lean: Option<LeanInfo>,
    pub mapped_by: Vec<u32>,
}

fn kind_of(magic: u64) -> Option<RingKind> {
    match magic {
        m if m == BROADCAST_RING_MAGIC => Some(RingKind::Broadcast),
        m if m == SPSC_RING_MAGIC => Some(RingKind::Spsc),
        m if m == MPMC_RING_MAGIC => Some(RingKind::Mpmc),
        m if m == crate::topic_rings::LEAN_RING_MAGIC => Some(RingKind::Lean),
        _ => None,
    }
}

pub(super) fn describe(path: &Path, maps: &Mappings) -> Option<RingInfo> {
    let name = path
        .file_name()?
        .to_str()?
        .strip_prefix(FILE_PREFIX)?
        .to_owned();
    let file = File::open(path).ok()?;
    let file_bytes = file.metadata().ok()?.len();
    let mut head = vec![0u8; LEAN_HEADER_BYTES];
    let got = file.read_at(&mut head, 0).ok()?;
    head.truncate(got);
    let magic = u64::from_ne_bytes(head.get(..8)?.try_into().ok()?);
    let kind = kind_of(magic)?;
    let lean = if kind == RingKind::Lean {
        inspect_lean_header(&head)
    } else {
        None
    };
    Some(RingInfo {
        name,
        path: path.to_owned(),
        kind,
        file_bytes,
        lean,
        mapped_by: maps.pids_for(path).to_vec(),
    })
}

pub fn scan_dir(dir: &Path) -> Vec<RingInfo> {
    let maps = mappings();
    let mut rings: Vec<RingInfo> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| describe(&entry.path(), &maps))
        .collect();
    rings.sort_by(|a, b| a.name.cmp(&b.name));
    rings
}

pub fn scan() -> Vec<RingInfo> {
    scan_dir(&shm_dir())
}
