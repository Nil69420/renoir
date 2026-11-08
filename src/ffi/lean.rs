use std::{
    ffi::{c_char, c_void, CStr},
    sync::Arc,
};

use crate::{
    memory::{BackingType, RegionConfig, SharedMemoryRegion},
    topic_rings::{lean_slot_bytes, LeanCursor, LeanPublisher, LeanRead, LeanRing},
};

use super::types::RenoirErrorCode;

pub type RenoirLeanHandle = *mut c_void;

pub const RENOIR_LEAN_FROM_OLDEST: u32 = 1;

struct LeanHandle {
    ring: Arc<LeanRing>,
    publisher: LeanPublisher,
    cursor: LeanCursor,
    _region: SharedMemoryRegion,
}

#[inline(always)]
fn handle<'a>(h: RenoirLeanHandle) -> Option<&'a mut LeanHandle> {
    unsafe { (h as *mut LeanHandle).as_mut() }
}

#[no_mangle]
pub extern "C" fn renoir_lean_open(
    name: *const c_char,
    msg_size: u32,
    slot_count: u32,
    flags: u32,
    out: *mut RenoirLeanHandle,
) -> RenoirErrorCode {
    if name.is_null() || out.is_null() || slot_count == 0 {
        return RenoirErrorCode::InvalidParameter;
    }
    let Ok(name) = unsafe { CStr::from_ptr(name) }.to_str() else {
        return RenoirErrorCode::InvalidParameter;
    };
    let msg = msg_size as usize;
    let Some(slot_bytes) = lean_slot_bytes(msg) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let Some(ring_bytes) = slot_bytes.checked_mul(slot_count as usize) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let size = LeanRing::required_memory_size(ring_bytes, msg);
    let config = RegionConfig {
        name: name.to_owned(),
        size,
        backing_type: BackingType::FileBacked,
        ..RegionConfig::default()
    };
    let region = match SharedMemoryRegion::new(config) {
        Ok(r) => r,
        Err(e) => return e.into(),
    };
    let Some(ptr) = std::ptr::NonNull::new(unsafe { region.as_mut_ptr_unsafe::<u8>() }) else {
        return RenoirErrorCode::OutOfMemory;
    };
    let ring = match unsafe { LeanRing::from_memory(ptr, region.size(), ring_bytes, msg) } {
        Ok(r) => Arc::new(r),
        Err(e) => return e.into(),
    };
    let cursor = if flags & RENOIR_LEAN_FROM_OLDEST != 0 {
        ring.cursor_oldest()
    } else {
        ring.cursor_latest()
    };
    let publisher = ring.publisher();
    let boxed = Box::new(LeanHandle {
        ring,
        publisher,
        cursor,
        _region: region,
    });
    unsafe { *out = Box::into_raw(boxed) as RenoirLeanHandle };
    RenoirErrorCode::Success
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
    match h.publisher.publish(src) {
        Ok(()) => RenoirErrorCode::Success,
        Err(e) => e.into(),
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
    match h.ring.try_read(&mut h.cursor, dst) {
        Ok(LeanRead::Message) => RenoirErrorCode::Success,
        Ok(LeanRead::Empty) => RenoirErrorCode::BufferEmpty,
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_skip_to_latest(h: RenoirLeanHandle, keep: u64) -> u64 {
    match handle(h) {
        Some(h) => h.ring.skip_to_latest(&mut h.cursor, keep),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_lost(h: RenoirLeanHandle) -> u64 {
    match handle(h) {
        Some(h) => h.cursor.lost(),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_pending(h: RenoirLeanHandle) -> usize {
    match handle(h) {
        Some(h) => h.ring.pending(&h.cursor),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_close(h: RenoirLeanHandle) {
    if !h.is_null() {
        drop(unsafe { Box::from_raw(h as *mut LeanHandle) });
    }
}
