use std::{
    ffi::{c_char, c_void, CStr},
    time::Duration,
};

use crate::topic_rings::{LeanChannel, LeanRead};

use super::types::RenoirErrorCode;

pub type RenoirLeanHandle = *mut c_void;

pub const RENOIR_LEAN_FROM_OLDEST: u32 = 1;

#[inline(always)]
fn handle<'a>(h: RenoirLeanHandle) -> Option<&'a mut LeanChannel> {
    unsafe { (h as *mut LeanChannel).as_mut() }
}

#[no_mangle]
pub extern "C" fn renoir_lean_open(
    name: *const c_char,
    msg_size: u32,
    slot_count: u32,
    flags: u32,
    out: *mut RenoirLeanHandle,
) -> RenoirErrorCode {
    if name.is_null() || out.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else {
        return RenoirErrorCode::InvalidParameter;
    };
    let from_oldest = flags & RENOIR_LEAN_FROM_OLDEST != 0;
    match LeanChannel::open(name, msg_size as usize, slot_count as usize, from_oldest) {
        Ok(channel) => {
            unsafe { *out = Box::into_raw(Box::new(channel)) as RenoirLeanHandle };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_publish(
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
    match h.publish(src) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_publish_notify(
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
    match h.publish_notify(src) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_wait(
    h: RenoirLeanHandle,
    spin_iterations: u32,
    timeout_ns: u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let timeout = (timeout_ns != u64::MAX).then(|| Duration::from_nanos(timeout_ns));
    if h.wait_readable(spin_iterations, timeout) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::BufferEmpty
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_try_read(
    h: RenoirLeanHandle,
    buffer: *mut u8,
    buffer_len: usize,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if buffer.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let dst = unsafe { std::slice::from_raw_parts_mut(buffer, buffer_len) };
    match h.try_read(dst) {
        Ok(LeanRead::Message) => RenoirErrorCode::Success,
        Ok(LeanRead::Empty) => RenoirErrorCode::BufferEmpty,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_skip_to_latest(h: RenoirLeanHandle, keep: u64) -> u64 {
    handle(h).map_or(0, |h| h.skip_to_latest(keep))
}

#[no_mangle]
pub extern "C" fn renoir_lean_lost(h: RenoirLeanHandle) -> u64 {
    handle(h).map_or(0, |h| h.lost())
}

#[no_mangle]
pub extern "C" fn renoir_lean_pending(h: RenoirLeanHandle) -> usize {
    handle(h).map_or(0, |h| h.pending())
}

#[no_mangle]
pub extern "C" fn renoir_lean_close(h: RenoirLeanHandle) {
    if !h.is_null() {
        drop(unsafe { Box::from_raw(h as *mut LeanChannel) });
    }
}
