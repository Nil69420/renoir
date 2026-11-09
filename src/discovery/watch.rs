use std::{
    collections::BTreeMap,
    fs,
    os::fd::{AsFd, AsRawFd, RawFd},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use nix::{
    poll::{poll, PollFd, PollFlags},
    sys::inotify::{AddWatchFlags, InitFlags, Inotify},
};

use super::procs::Mappings;
use super::scan::{describe, RingInfo, FILE_PREFIX};
use crate::error::{RenoirError, Result};
use crate::memory::config::shm_dir;

const FAST_RESCAN: Duration = Duration::from_millis(5);
const UNREADY_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Appeared(RingInfo),
    Disappeared {
        name: String,
    },
    Changed {
        previous: RingInfo,
        current: RingInfo,
    },
}

pub struct DiscoveryWatcher {
    dir: PathBuf,
    inotify: Inotify,
    known: BTreeMap<String, RingInfo>,
    interval: Duration,
    next_rescan: Instant,
    unready_since: Option<Instant>,
}

fn signature(info: &RingInfo) -> (u64, usize, usize, u64) {
    let (published_slots, live, consumers) = info.lean.as_ref().map_or((0, 0, 0), |l| {
        (l.slot_count, l.live_publishers, l.registered_consumers)
    });
    (published_slots, live, consumers, info.file_bytes)
}

fn io_error(error: impl std::fmt::Display) -> RenoirError {
    RenoirError::platform(format!("discovery watch: {error}"))
}

impl DiscoveryWatcher {
    pub fn open_dir(dir: &Path, interval: Duration) -> Result<Self> {
        let inotify =
            Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).map_err(io_error)?;
        inotify
            .add_watch(
                dir,
                AddWatchFlags::IN_CREATE
                    | AddWatchFlags::IN_DELETE
                    | AddWatchFlags::IN_MOVED_TO
                    | AddWatchFlags::IN_MOVED_FROM
                    | AddWatchFlags::IN_MODIFY,
            )
            .map_err(io_error)?;
        let mut watcher = Self {
            dir: dir.to_owned(),
            inotify,
            known: BTreeMap::new(),
            interval,
            next_rescan: Instant::now(),
            unready_since: None,
        };
        let (rings, unready) = watcher.scan_now();
        watcher.known = rings.into_iter().map(|r| (r.name.clone(), r)).collect();
        let now = Instant::now();
        if unready > 0 {
            watcher.unready_since = Some(now);
            watcher.next_rescan = now + FAST_RESCAN;
        } else {
            watcher.next_rescan = now + interval;
        }
        Ok(watcher)
    }

    pub fn open(interval: Duration) -> Result<Self> {
        Self::open_dir(&shm_dir(), interval)
    }

    pub fn fd(&self) -> RawFd {
        self.inotify.as_fd().as_raw_fd()
    }

    pub fn snapshot(&self) -> Vec<RingInfo> {
        self.known.values().cloned().collect()
    }

    fn scan_now(&self) -> (Vec<RingInfo>, usize) {
        let maps = Mappings::default();
        let mut rings = Vec::new();
        let mut unready = 0usize;
        for entry in fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let is_ring_file = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(FILE_PREFIX));
            if !is_ring_file {
                continue;
            }
            match describe(&path, &maps) {
                Some(info) => rings.push(info),
                None => unready += 1,
            }
        }
        (rings, unready)
    }

    fn drain_inotify(&self) -> bool {
        let mut any = false;
        while let Ok(events) = self.inotify.read_events() {
            if events.is_empty() {
                break;
            }
            any = true;
        }
        any
    }

    fn diff(&mut self, rings: Vec<RingInfo>) -> Vec<DiscoveryEvent> {
        let mut events = Vec::new();
        let mut current: BTreeMap<String, RingInfo> =
            rings.into_iter().map(|r| (r.name.clone(), r)).collect();
        for (name, previous) in &self.known {
            if !current.contains_key(name) {
                events.push(DiscoveryEvent::Disappeared { name: name.clone() });
            } else if signature(previous) != signature(&current[name]) {
                events.push(DiscoveryEvent::Changed {
                    previous: previous.clone(),
                    current: current[name].clone(),
                });
            }
        }
        for (name, info) in &current {
            if !self.known.contains_key(name) {
                events.push(DiscoveryEvent::Appeared(info.clone()));
            }
        }
        std::mem::swap(&mut self.known, &mut current);
        events
    }

    pub fn poll(&mut self, timeout: Duration) -> Vec<DiscoveryEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            let wake = self.next_rescan.min(deadline);
            let wait = wake.saturating_duration_since(now);
            let mut dirty = false;
            if wait > Duration::ZERO {
                let mut fds = [PollFd::new(&self.inotify, PollFlags::POLLIN)];
                let millis = i32::try_from(wait.as_millis().max(1)).unwrap_or(i32::MAX);
                if matches!(poll(&mut fds, millis), Ok(n) if n > 0) {
                    dirty = true;
                }
            }
            dirty |= self.drain_inotify();
            if dirty || Instant::now() >= self.next_rescan {
                let (rings, unready) = self.scan_now();
                let now = Instant::now();
                self.unready_since = if unready > 0 {
                    Some(self.unready_since.unwrap_or(now))
                } else {
                    None
                };
                let fast = self
                    .unready_since
                    .is_some_and(|since| now.duration_since(since) < UNREADY_GRACE);
                self.next_rescan = now + if fast { FAST_RESCAN } else { self.interval };
                let events = self.diff(rings);
                if !events.is_empty() {
                    return events;
                }
            }
            if Instant::now() >= deadline {
                return Vec::new();
            }
        }
    }
}
