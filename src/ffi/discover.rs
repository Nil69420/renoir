use std::{collections::VecDeque, ffi::c_char, ffi::c_void, time::Duration};

use crate::discovery::{
    scan, sweep, DiscoveryEvent, DiscoveryWatcher, RingInfo, RingKind, SweepOptions,
};
use crate::topic_rings::LeanMode;

use super::types::RenoirErrorCode;

pub const RENOIR_RING_NAME_BYTES: usize = 64;
pub const RENOIR_SWEEP_DRY_RUN: u32 = 1;
pub const RENOIR_SWEEP_IGNORE_UNREADABLE: u32 = 2;
pub const RENOIR_RING_KIND_LEAN: u32 = 0;
pub const RENOIR_RING_KIND_BROADCAST: u32 = 1;
pub const RENOIR_RING_KIND_SPSC: u32 = 2;
pub const RENOIR_RING_KIND_MPMC: u32 = 3;
pub const RENOIR_RING_MODE_NONE: u32 = u32::MAX;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RenoirRingInfo {
    pub name: [c_char; RENOIR_RING_NAME_BYTES],
    pub kind: u32,
    pub mode: u32,
    pub msg_size: u32,
    pub slot_count: u32,
    pub published: u64,
    pub file_bytes: u64,
    pub live_publishers: u32,
    pub registered_consumers: u32,
    pub mapped_by_count: u32,
    pub reserved: u32,
}

fn convert(ring: &RingInfo) -> RenoirRingInfo {
    let mut name = [0 as c_char; RENOIR_RING_NAME_BYTES];
    for (dst, src) in name
        .iter_mut()
        .zip(ring.name.bytes().take(RENOIR_RING_NAME_BYTES - 1))
    {
        *dst = src as c_char;
    }
    let (mode, msg_size, slot_count, published, live_publishers, registered_consumers) =
        match &ring.lean {
            Some(l) => (
                match l.mode {
                    LeanMode::SingleProducer => 0,
                    LeanMode::MultiProducer => 1,
                    LeanMode::SingleProducerGated => 2,
                    LeanMode::MultiProducerGated => 3,
                },
                u32::try_from(l.msg_size).unwrap_or(u32::MAX),
                u32::try_from(l.slot_count).unwrap_or(u32::MAX),
                l.published,
                u32::try_from(l.live_publishers).unwrap_or(u32::MAX),
                u32::try_from(l.registered_consumers).unwrap_or(u32::MAX),
            ),
            None => (RENOIR_RING_MODE_NONE, 0, 0, 0, 0, 0),
        };
    RenoirRingInfo {
        name,
        kind: match ring.kind {
            RingKind::Lean => RENOIR_RING_KIND_LEAN,
            RingKind::Broadcast => RENOIR_RING_KIND_BROADCAST,
            RingKind::Spsc => RENOIR_RING_KIND_SPSC,
            RingKind::Mpmc => RENOIR_RING_KIND_MPMC,
        },
        mode,
        msg_size,
        slot_count,
        published,
        file_bytes: ring.file_bytes,
        live_publishers,
        registered_consumers,
        mapped_by_count: u32::try_from(ring.mapped_by.len()).unwrap_or(u32::MAX),
        reserved: 0,
    }
}

#[no_mangle]
pub extern "C" fn renoir_discover(
    out: *mut RenoirRingInfo,
    capacity: usize,
    total: *mut usize,
) -> RenoirErrorCode {
    if total.is_null() || (out.is_null() && capacity != 0) {
        return RenoirErrorCode::InvalidParameter;
    }
    let rings = scan();
    unsafe { *total = rings.len() };
    for (i, ring) in rings.iter().take(capacity).enumerate() {
        unsafe { out.add(i).write(convert(ring)) };
    }
    RenoirErrorCode::Success
}

#[no_mangle]
pub extern "C" fn renoir_discover_sweep(flags: u32, removed: *mut usize) -> RenoirErrorCode {
    let report = sweep(SweepOptions {
        dry_run: flags & RENOIR_SWEEP_DRY_RUN != 0,
        ignore_unreadable_processes: flags & RENOIR_SWEEP_IGNORE_UNREADABLE != 0,
    });
    if !removed.is_null() {
        unsafe { *removed = report.removed.len() };
    }
    RenoirErrorCode::Success
}

pub const RENOIR_DISCOVERY_APPEARED: u32 = 0;
pub const RENOIR_DISCOVERY_DISAPPEARED: u32 = 1;
pub const RENOIR_DISCOVERY_CHANGED: u32 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RenoirDiscoveryEvent {
    pub kind: u32,
    pub previous_live_publishers: u32,
    pub previous_registered_consumers: u32,
    pub reserved: u32,
    pub ring: RenoirRingInfo,
}

pub type RenoirDiscoveryWatchHandle = *mut c_void;

struct Watch {
    watcher: DiscoveryWatcher,
    pending: VecDeque<RenoirDiscoveryEvent>,
}

fn lean_counts(ring: &RingInfo) -> (u32, u32) {
    ring.lean.as_ref().map_or((0, 0), |l| {
        (
            u32::try_from(l.live_publishers).unwrap_or(u32::MAX),
            u32::try_from(l.registered_consumers).unwrap_or(u32::MAX),
        )
    })
}

fn convert_event(event: &DiscoveryEvent) -> RenoirDiscoveryEvent {
    match event {
        DiscoveryEvent::Appeared(ring) => RenoirDiscoveryEvent {
            kind: RENOIR_DISCOVERY_APPEARED,
            previous_live_publishers: 0,
            previous_registered_consumers: 0,
            reserved: 0,
            ring: convert(ring),
        },
        DiscoveryEvent::Disappeared { name } => {
            let mut ring = convert(&RingInfo {
                name: name.clone(),
                path: std::path::PathBuf::new(),
                kind: RingKind::Lean,
                file_bytes: 0,
                lean: None,
                mapped_by: Vec::new(),
            });
            ring.kind = RENOIR_RING_KIND_LEAN;
            RenoirDiscoveryEvent {
                kind: RENOIR_DISCOVERY_DISAPPEARED,
                previous_live_publishers: 0,
                previous_registered_consumers: 0,
                reserved: 0,
                ring,
            }
        }
        DiscoveryEvent::Changed { previous, current } => {
            let (publishers, consumers) = lean_counts(previous);
            RenoirDiscoveryEvent {
                kind: RENOIR_DISCOVERY_CHANGED,
                previous_live_publishers: publishers,
                previous_registered_consumers: consumers,
                reserved: 0,
                ring: convert(current),
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn renoir_discover_watch_open(
    interval_ms: u32,
    out: *mut RenoirDiscoveryWatchHandle,
) -> RenoirErrorCode {
    if out.is_null() || interval_ms == 0 {
        return RenoirErrorCode::InvalidParameter;
    }
    match DiscoveryWatcher::open(Duration::from_millis(u64::from(interval_ms))) {
        Ok(watcher) => {
            let watch = Box::new(Watch {
                watcher,
                pending: VecDeque::new(),
            });
            unsafe { *out = Box::into_raw(watch).cast() };
            RenoirErrorCode::Success
        }
        Err(error) => error.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_discover_watch_fd(handle: RenoirDiscoveryWatchHandle) -> i32 {
    if handle.is_null() {
        return -1;
    }
    let watch = unsafe { &*handle.cast::<Watch>() };
    watch.watcher.fd()
}

#[no_mangle]
pub extern "C" fn renoir_discover_watch_poll(
    handle: RenoirDiscoveryWatchHandle,
    timeout_ms: u32,
    out: *mut RenoirDiscoveryEvent,
    capacity: usize,
    count: *mut usize,
) -> RenoirErrorCode {
    if handle.is_null() || count.is_null() || (out.is_null() && capacity != 0) {
        return RenoirErrorCode::InvalidParameter;
    }
    let watch = unsafe { &mut *handle.cast::<Watch>() };
    if watch.pending.is_empty() {
        let events = watch
            .watcher
            .poll(Duration::from_millis(u64::from(timeout_ms)));
        watch.pending.extend(events.iter().map(convert_event));
    }
    let mut written = 0;
    while written < capacity {
        let Some(event) = watch.pending.pop_front() else {
            break;
        };
        unsafe { out.add(written).write(event) };
        written += 1;
    }
    unsafe { *count = written };
    RenoirErrorCode::Success
}

#[no_mangle]
pub extern "C" fn renoir_discover_watch_close(handle: RenoirDiscoveryWatchHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle.cast::<Watch>()) });
    }
}
