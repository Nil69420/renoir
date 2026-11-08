use std::{
    fs,
    path::{Path, PathBuf},
};

use super::procs::mappings;
use super::scan::{describe, FILE_PREFIX};
use crate::memory::config::shm_dir;

#[derive(Debug, Clone, Copy, Default)]
pub struct SweepOptions {
    pub dry_run: bool,
    pub ignore_unreadable_processes: bool,
}

#[derive(Debug, Default)]
pub struct SweepReport {
    pub removed: Vec<PathBuf>,
    pub kept_in_use: Vec<PathBuf>,
    pub kept_unverifiable: Vec<PathBuf>,
}

pub fn sweep_dir(dir: &Path, options: SweepOptions) -> SweepReport {
    let maps = mappings();
    let mut report = SweepReport::default();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let is_ring_file = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(FILE_PREFIX));
        if !is_ring_file || describe(&path, &maps).is_none() {
            continue;
        }
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if !maps.pids_for(&path).is_empty() {
            report.kept_in_use.push(path);
        } else if !options.ignore_unreadable_processes && maps.could_be_hidden(&meta) {
            report.kept_unverifiable.push(path);
        } else if options.dry_run || fs::remove_file(&path).is_ok() {
            report.removed.push(path);
        }
    }
    report
}

pub fn sweep(options: SweepOptions) -> SweepReport {
    sweep_dir(&shm_dir(), options)
}
