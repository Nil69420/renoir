use std::ffi::c_char;

use crate::discovery::{scan, sweep, RingInfo, RingKind, SweepOptions};
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
