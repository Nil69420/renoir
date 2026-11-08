use std::{
    ffi::{c_char, c_void, CStr},
    time::Duration,
};

use crate::topic_rings::FrameChannel;

use super::{
    lean::{RENOIR_LEAN_CONSUMER, RENOIR_LEAN_FROM_OLDEST, RENOIR_LEAN_MULTI_PRODUCER},
    types::RenoirErrorCode,
};

pub type RenoirFrameHandle = *mut c_void;

#[inline(always)]
fn handle<'a>(h: RenoirFrameHandle) -> Option<&'a mut FrameChannel> {
    unsafe { (h as *mut FrameChannel).as_mut() }
}

#[no_mangle]
pub extern "C" fn renoir_frame_open(
    name: *const c_char,
    max_frame: usize,
    slot_count: u32,
    flags: u32,
    out: *mut RenoirFrameHandle,
) -> RenoirErrorCode {
    if name.is_null() || out.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else {
        return RenoirErrorCode::InvalidParameter;
    };
    match FrameChannel::open(
        name,
        max_frame,
        slot_count as usize,
        flags & RENOIR_LEAN_MULTI_PRODUCER != 0,
        flags & RENOIR_LEAN_CONSUMER != 0,
        flags & RENOIR_LEAN_FROM_OLDEST != 0,
    ) {
        Ok(channel) => {
            unsafe { *out = Box::into_raw(Box::new(channel)) as RenoirFrameHandle };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_loan(
    h: RenoirFrameHandle,
    len: usize,
    out_data: *mut *mut u8,
) -> RenoirErrorCode {
    let (Some(h), false) = (handle(h), out_data.is_null()) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match h.loan_raw(len) {
        Ok(ptr) => {
            unsafe { *out_data = ptr };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_commit(h: RenoirFrameHandle) -> RenoirErrorCode {
    match handle(h) {
        Some(h) => h
            .commit_raw(false)
            .map_or_else(Into::into, |()| RenoirErrorCode::Success),
        None => RenoirErrorCode::InvalidParameter,
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_commit_notify(h: RenoirFrameHandle) -> RenoirErrorCode {
    match handle(h) {
        Some(h) => h
            .commit_raw(true)
            .map_or_else(Into::into, |()| RenoirErrorCode::Success),
        None => RenoirErrorCode::InvalidParameter,
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_abort(h: RenoirFrameHandle) -> RenoirErrorCode {
    match handle(h) {
        Some(h) => h
            .abort_raw()
            .map_or_else(Into::into, |()| RenoirErrorCode::Success),
        None => RenoirErrorCode::InvalidParameter,
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_view(
    h: RenoirFrameHandle,
    out_data: *mut *const u8,
    out_len: *mut usize,
) -> RenoirErrorCode {
    let (Some(h), false, false) = (handle(h), out_data.is_null(), out_len.is_null()) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match h.view_raw() {
        Ok(Some((ptr, len))) => {
            unsafe {
                *out_data = ptr;
                *out_len = len;
            }
            RenoirErrorCode::Success
        }
        Ok(None) => RenoirErrorCode::BufferEmpty,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_release(h: RenoirFrameHandle) -> RenoirErrorCode {
    match handle(h) {
        Some(h) => h
            .release_raw()
            .map_or_else(Into::into, |()| RenoirErrorCode::Success),
        None => RenoirErrorCode::InvalidParameter,
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_view_intact(h: RenoirFrameHandle) -> bool {
    handle(h).is_some_and(|h| h.view_intact_raw())
}

#[no_mangle]
pub extern "C" fn renoir_frame_wait(
    h: RenoirFrameHandle,
    spin_iterations: u32,
    timeout_ns: u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let timeout = (timeout_ns != u64::MAX).then(|| Duration::from_nanos(timeout_ns));
    if h.wait(spin_iterations, timeout) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::BufferEmpty
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_consumer_id(h: RenoirFrameHandle) -> i32 {
    handle(h)
        .and_then(|h| h.consumer_id())
        .map_or(-1, |id| id as i32)
}

#[no_mangle]
pub extern "C" fn renoir_frame_set_upstream(
    h: RenoirFrameHandle,
    upstream: u32,
) -> RenoirErrorCode {
    match handle(h) {
        Some(h) => h
            .set_upstream(upstream as usize)
            .map_or_else(Into::into, |()| RenoirErrorCode::Success),
        None => RenoirErrorCode::InvalidParameter,
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_live_publishers(h: RenoirFrameHandle) -> usize {
    handle(h).map_or(0, |h| h.live_publishers())
}

#[no_mangle]
pub extern "C" fn renoir_frame_max_frame(h: RenoirFrameHandle) -> usize {
    handle(h).map_or(0, |h| h.max_frame())
}

#[no_mangle]
pub extern "C" fn renoir_frame_close(h: RenoirFrameHandle) {
    if !h.is_null() {
        drop(unsafe { Box::from_raw(h as *mut FrameChannel) });
    }
}
