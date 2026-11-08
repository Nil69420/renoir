use std::{
    collections::HashMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Debug, Default)]
pub(super) struct Mappings {
    pub(super) by_path: HashMap<PathBuf, Vec<u32>>,
    pub(super) unreadable_uids: Vec<u32>,
}

pub(super) fn mappings() -> Mappings {
    let mut out = Mappings::default();
    let Ok(entries) = fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        match fs::read_to_string(entry.path().join("maps")) {
            Ok(text) => {
                for line in text.lines() {
                    let Some(path) = line.split_whitespace().nth(5) else {
                        continue;
                    };
                    if path.starts_with('/') {
                        let list = out.by_path.entry(PathBuf::from(path)).or_default();
                        if !list.contains(&pid) {
                            list.push(pid);
                        }
                    }
                }
            }
            Err(_) => {
                if let Ok(meta) = fs::metadata(entry.path()) {
                    out.unreadable_uids.push(meta.uid());
                }
            }
        }
    }
    out
}

impl Mappings {
    pub(super) fn pids_for(&self, path: &Path) -> &[u32] {
        self.by_path.get(path).map_or(&[], Vec::as_slice)
    }

    pub(super) fn could_be_hidden(&self, file: &fs::Metadata) -> bool {
        let shared = file.mode() & 0o066 != 0;
        self.unreadable_uids
            .iter()
            .any(|uid| shared || *uid == file.uid())
    }
}
