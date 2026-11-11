use std::{ffi::c_char, ffi::CStr, time::Duration};

use crate::topic_rings::{DeadlineWait, FrameChannel};

use super::{
    frame::{handle, RenoirFrameHandle},
    lean::{RENOIR_LEAN_CONSUMER, RENOIR_LEAN_MULTI_PRODUCER},
    lean_qos::{convert, fill_status, RenoirLeanQos, RenoirLivelinessStatus},
    types::RenoirErrorCode,
};

#[no_mangle]
pub extern "C" fn renoir_frame_open_qos(
    name: *const c_char,
    max_frame: usize,
    qos: *const RenoirLeanQos,
    flags: u32,
    out: *mut RenoirFrameHandle,
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
    match FrameChannel::open_qos(
        name,
        max_frame,
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
pub extern "C" fn renoir_frame_register_publisher(h: RenoirFrameHandle) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match h.register_publisher() {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_assert_liveliness(h: RenoirFrameHandle) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match h.assert_liveliness() {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_liveliness(
    h: RenoirFrameHandle,
    out: *mut RenoirLivelinessStatus,
) -> RenoirErrorCode {
    let (Some(h), false) = (handle(h), out.is_null()) else {
        return RenoirErrorCode::InvalidParameter;
    };
    fill_status(h.liveliness(), out);
    RenoirErrorCode::Success
}

#[no_mangle]
pub extern "C" fn renoir_frame_wait_liveliness_change(
    h: RenoirFrameHandle,
    timeout_ns: u64,
    out: *mut RenoirLivelinessStatus,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let timeout = (timeout_ns != u64::MAX).then(|| Duration::from_nanos(timeout_ns));
    match h.wait_liveliness_change(timeout) {
        Some(status) => {
            fill_status(status, out);
            RenoirErrorCode::Success
        }
        None => RenoirErrorCode::BufferEmpty,
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_is_owner(h: RenoirFrameHandle) -> i32 {
    handle(h).map_or(-1, |h| i32::from(h.is_owner()))
}

#[no_mangle]
pub extern "C" fn renoir_frame_view_qos(
    h: RenoirFrameHandle,
    out_data: *mut *const u8,
    out_len: *mut usize,
) -> RenoirErrorCode {
    let (Some(h), false, false) = (handle(h), out_data.is_null(), out_len.is_null()) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match h.view_qos_raw() {
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
pub extern "C" fn renoir_frame_wait_deadline(
    h: RenoirFrameHandle,
    spin_iterations: u32,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    match h.wait_deadline(spin_iterations) {
        DeadlineWait::Ready => RenoirErrorCode::Success,
        DeadlineWait::Missed => RenoirErrorCode::BufferEmpty,
    }
}

#[no_mangle]
pub extern "C" fn renoir_frame_deadline_misses(h: RenoirFrameHandle) -> u64 {
    handle(h).map_or(0, |h| h.deadline_misses())
}
