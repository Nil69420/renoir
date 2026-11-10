use std::time::Duration;

use crate::topic_rings::{LeanRead, LeanWaitStrategy};

use super::{
    lean::{
        handle, renoir_lean_memory_required, RenoirLeanHandle, RENOIR_LEAN_CLOCK_NONE,
        RENOIR_LEAN_STAMP_NS, RENOIR_LEAN_STAMP_TICKS,
    },
    types::RenoirErrorCode,
};

#[no_mangle]
pub extern "C" fn renoir_lean_publish_at(
    h: RenoirLeanHandle,
    stamp: u64,
    data: *const u8,
    len: usize,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if data.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let src = unsafe { std::slice::from_raw_parts(data, len) };
    match h.publish_at(stamp, src) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_publish_now(
    h: RenoirLeanHandle,
    data: *const u8,
    len: usize,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if data.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let src = unsafe { std::slice::from_raw_parts(data, len) };
    match h.publish_now(src) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_try_read_stamped(
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
    match h.try_read_stamped(dst) {
        Ok(Some(stamp)) => {
            unsafe { *out_stamp = stamp };
            RenoirErrorCode::Success
        }
        Ok(None) => RenoirErrorCode::BufferEmpty,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_clock_kind(h: RenoirLeanHandle) -> u32 {
    handle(h).map_or(RENOIR_LEAN_CLOCK_NONE, |h| h.ring().clock() as u32)
}

#[no_mangle]
pub extern "C" fn renoir_lean_tick_hz(h: RenoirLeanHandle) -> u64 {
    handle(h).map_or(0, |h| h.ring().tick_hz())
}

#[no_mangle]
pub extern "C" fn renoir_lean_stamp_now(h: RenoirLeanHandle) -> u64 {
    handle(h).map_or(0, |h| h.ring().stamp_now())
}

#[no_mangle]
pub extern "C" fn renoir_lean_stamp_to_ns(h: RenoirLeanHandle, stamp: u64) -> u64 {
    handle(h).map_or(0, |h| h.ring().ticks_to_ns(stamp))
}

#[no_mangle]
pub extern "C" fn renoir_lean_memory_required_flags(
    msg_size: u32,
    slot_count: u32,
    flags: u32,
    out_bytes: *mut usize,
) -> RenoirErrorCode {
    let stamped = flags & (RENOIR_LEAN_STAMP_NS | RENOIR_LEAN_STAMP_TICKS) != 0;
    let wire = if stamped {
        let Some(wire) = msg_size.checked_add(crate::topic_rings::STAMP_BYTES as u32) else {
            return RenoirErrorCode::InvalidParameter;
        };
        wire
    } else {
        msg_size
    };
    renoir_lean_memory_required(wire, slot_count, out_bytes)
}

#[no_mangle]
pub extern "C" fn renoir_lean_try_read_lagged(
    h: RenoirLeanHandle,
    buffer: *mut u8,
    buffer_len: usize,
    out_missed: *mut u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if buffer.is_null() || out_missed.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(buffer, buffer_len) };
    match h.try_read_lagged(dst) {
        Ok((read, missed)) => {
            unsafe { *out_missed = missed };
            match read {
                LeanRead::Message => RenoirErrorCode::Success,
                LeanRead::Empty => RenoirErrorCode::BufferEmpty,
            }
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_try_read_stamped_lagged(
    h: RenoirLeanHandle,
    buffer: *mut u8,
    buffer_len: usize,
    out_stamp: *mut u64,
    out_missed: *mut u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if buffer.is_null() || out_stamp.is_null() || out_missed.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(buffer, buffer_len) };
    match h.try_read_stamped_lagged(dst) {
        Ok((stamp, missed)) => {
            unsafe { *out_missed = missed };
            match stamp {
                Some(stamp) => {
                    unsafe { *out_stamp = stamp };
                    RenoirErrorCode::Success
                }
                None => RenoirErrorCode::BufferEmpty,
            }
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_wait_strategy(
    h: RenoirLeanHandle,
    strategy: u32,
    timeout_ns: u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let Some(strategy) = LeanWaitStrategy::from_raw(strategy) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let timeout = (timeout_ns != u64::MAX).then(|| Duration::from_nanos(timeout_ns));
    if h.wait_with(strategy, timeout) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::BufferEmpty
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_wait_phased(
    h: RenoirLeanHandle,
    spin_ns: u64,
    yield_ns: u64,
    timeout_ns: u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let timeout = (timeout_ns != u64::MAX).then(|| Duration::from_nanos(timeout_ns));
    if h.wait_phased(
        Duration::from_nanos(spin_ns),
        Duration::from_nanos(yield_ns),
        timeout,
    ) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::BufferEmpty
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_publish_at_notify(
    h: RenoirLeanHandle,
    stamp: u64,
    data: *const u8,
    len: usize,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if data.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let src = unsafe { std::slice::from_raw_parts(data, len) };
    match h.publish_at_notify(stamp, src) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_publish_now_notify(
    h: RenoirLeanHandle,
    data: *const u8,
    len: usize,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if data.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let src = unsafe { std::slice::from_raw_parts(data, len) };
    match h.publish_now_notify(src) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}
