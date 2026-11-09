use std::{ffi::c_char, ffi::CStr, time::Duration};

use crate::topic_rings::{
    LeanChannel, LeanClock, LeanDurability, LeanHistory, LeanLiveliness, LeanLivelinessKind,
    LeanOwnership, LeanQos, LeanReliability,
};

use super::{
    lean::{
        handle, RenoirLeanHandle, RENOIR_LEAN_CLOCK_NS, RENOIR_LEAN_CLOCK_TICKS,
        RENOIR_LEAN_CONSUMER, RENOIR_LEAN_MULTI_PRODUCER,
    },
    types::RenoirErrorCode,
};

pub const RENOIR_QOS_BEST_EFFORT: u32 = 0;
pub const RENOIR_QOS_RELIABLE: u32 = 1;
pub const RENOIR_QOS_KEEP_LAST: u32 = 0;
pub const RENOIR_QOS_KEEP_ALL: u32 = 1;
pub const RENOIR_QOS_VOLATILE: u32 = 0;
pub const RENOIR_QOS_TRANSIENT_LOCAL: u32 = 1;
pub const RENOIR_QOS_LIVELINESS_AUTOMATIC: u32 = 0;
pub const RENOIR_QOS_LIVELINESS_MANUAL: u32 = 1;
pub const RENOIR_QOS_OWNERSHIP_SHARED: u32 = 0;
pub const RENOIR_QOS_OWNERSHIP_EXCLUSIVE: u32 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RenoirLeanQos {
    pub reliability: u32,
    pub history: u32,
    pub durability: u32,
    pub depth: u32,
    pub deadline_ns: u64,
    pub lifespan_ns: u64,
    pub min_separation_ns: u64,
    pub liveliness_kind: u32,
    pub ownership: u32,
    pub liveliness_lease_ns: u64,
    pub ownership_strength: u64,
    pub clock: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RenoirLivelinessStatus {
    pub registered: u32,
    pub alive: u32,
    pub lost: u32,
    pub reserved: u32,
    pub lost_count: u64,
}

fn optional(ns: u64) -> Option<Duration> {
    (ns != 0).then(|| Duration::from_nanos(ns))
}

fn convert(q: &RenoirLeanQos) -> Option<LeanQos> {
    let reliability = match q.reliability {
        RENOIR_QOS_BEST_EFFORT => LeanReliability::BestEffort,
        RENOIR_QOS_RELIABLE => LeanReliability::Reliable,
        _ => return None,
    };
    let history = match q.history {
        RENOIR_QOS_KEEP_LAST => LeanHistory::KeepLast,
        RENOIR_QOS_KEEP_ALL => LeanHistory::KeepAll,
        _ => return None,
    };
    let durability = match q.durability {
        RENOIR_QOS_VOLATILE => LeanDurability::Volatile,
        RENOIR_QOS_TRANSIENT_LOCAL => LeanDurability::TransientLocal,
        _ => return None,
    };
    let kind = match q.liveliness_kind {
        RENOIR_QOS_LIVELINESS_AUTOMATIC => LeanLivelinessKind::Automatic,
        RENOIR_QOS_LIVELINESS_MANUAL => LeanLivelinessKind::Manual,
        _ => return None,
    };
    let ownership = match q.ownership {
        RENOIR_QOS_OWNERSHIP_SHARED => LeanOwnership::Shared,
        RENOIR_QOS_OWNERSHIP_EXCLUSIVE => LeanOwnership::Exclusive {
            strength: q.ownership_strength,
        },
        _ => return None,
    };
    let clock = match q.clock {
        0 => LeanClock::None,
        RENOIR_LEAN_CLOCK_NS => LeanClock::Nanoseconds,
        RENOIR_LEAN_CLOCK_TICKS => LeanClock::Ticks,
        _ => return None,
    };
    Some(LeanQos {
        reliability,
        depth: q.depth as usize,
        deadline: optional(q.deadline_ns),
        history,
        durability,
        lifespan: optional(q.lifespan_ns),
        liveliness: LeanLiveliness {
            kind,
            lease: optional(q.liveliness_lease_ns),
        },
        ownership,
        min_separation: optional(q.min_separation_ns),
        clock,
    })
}

#[no_mangle]
pub extern "C" fn renoir_lean_qos_default(out: *mut RenoirLeanQos) -> RenoirErrorCode {
    if out.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    unsafe {
        *out = RenoirLeanQos {
            reliability: RENOIR_QOS_BEST_EFFORT,
            history: RENOIR_QOS_KEEP_LAST,
            durability: RENOIR_QOS_VOLATILE,
            depth: 64,
            deadline_ns: 0,
            lifespan_ns: 0,
            min_separation_ns: 0,
            liveliness_kind: RENOIR_QOS_LIVELINESS_AUTOMATIC,
            ownership: RENOIR_QOS_OWNERSHIP_SHARED,
            liveliness_lease_ns: 0,
            ownership_strength: 0,
            clock: 0,
            reserved: 0,
        };
    }
    RenoirErrorCode::Success
}

#[no_mangle]
pub extern "C" fn renoir_lean_open_qos(
    name: *const c_char,
    msg_size: u32,
    qos: *const RenoirLeanQos,
    flags: u32,
    out: *mut RenoirLeanHandle,
) -> RenoirErrorCode {
    if name.is_null() || qos.is_null() || out.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else {
        return RenoirErrorCode::InvalidParameter;
    };
    let Some(qos) = convert(unsafe { &*qos }) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match LeanChannel::open_qos(
        name,
        msg_size as usize,
        qos,
        flags & RENOIR_LEAN_MULTI_PRODUCER != 0,
        flags & RENOIR_LEAN_CONSUMER != 0,
    ) {
        Ok(channel) => {
            unsafe { *out = Box::into_raw(Box::new(channel)).cast() };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_register_publisher(h: RenoirLeanHandle) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match h.register_publisher() {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_assert_liveliness(h: RenoirLeanHandle) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match h.assert_liveliness() {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_liveliness(
    h: RenoirLeanHandle,
    out: *mut RenoirLivelinessStatus,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if out.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let status = h.liveliness();
    unsafe {
        *out = RenoirLivelinessStatus {
            registered: u32::try_from(status.registered).unwrap_or(u32::MAX),
            alive: u32::try_from(status.alive).unwrap_or(u32::MAX),
            lost: u32::from(status.lost),
            reserved: 0,
            lost_count: status.lost_count,
        };
    }
    RenoirErrorCode::Success
}

#[no_mangle]
pub extern "C" fn renoir_lean_is_owner(h: RenoirLeanHandle) -> i32 {
    handle(h).map_or(-1, |h| i32::from(h.is_owner()))
}

#[no_mangle]
pub extern "C" fn renoir_lean_try_read_stamped_qos(
    h: RenoirLeanHandle,
    buffer: *mut u8,
    buffer_len: usize,
    out_stamp: *mut u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if buffer.is_null() || out_stamp.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(buffer, buffer_len) };
    match h.try_read_stamped_qos(dst) {
        Ok(Some(stamp)) => {
            unsafe { *out_stamp = stamp };
            RenoirErrorCode::Success
        }
        Ok(None) => RenoirErrorCode::BufferEmpty,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_stale_dropped(h: RenoirLeanHandle) -> u64 {
    handle(h).map_or(0, |h| h.stale_dropped())
}

#[no_mangle]
pub extern "C" fn renoir_lean_filtered_out(h: RenoirLeanHandle) -> u64 {
    handle(h).map_or(0, |h| h.filtered_out())
}
