use std::{
    ffi::{c_char, c_void, CStr},
    time::Duration,
};

use crate::topic_rings::{DeadlineWait, LeanChannel, LeanClock, LeanMode, LeanRead};

use super::types::RenoirErrorCode;

pub type RenoirLeanHandle = *mut c_void;

pub const RENOIR_LEAN_FROM_OLDEST: u32 = 1;
pub const RENOIR_LEAN_MULTI_PRODUCER: u32 = 2;
pub const RENOIR_LEAN_GATED: u32 = 4;
pub const RENOIR_LEAN_CONSUMER: u32 = 8;
pub const RENOIR_LEAN_STAMP_NS: u32 = 16;
pub const RENOIR_LEAN_STAMP_TICKS: u32 = 32;
pub const RENOIR_LEAN_CLOCK_NONE: u32 = 0;
pub const RENOIR_LEAN_CLOCK_NS: u32 = 1;
pub const RENOIR_LEAN_CLOCK_TICKS: u32 = 2;
pub const RENOIR_LEAN_RELIABLE_PUBLISHER: u32 = RENOIR_LEAN_GATED;
pub const RENOIR_LEAN_RELIABLE_SUBSCRIBER: u32 = RENOIR_LEAN_GATED | RENOIR_LEAN_CONSUMER;

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
    let mode = LeanMode::new(
        flags & RENOIR_LEAN_MULTI_PRODUCER != 0,
        flags & RENOIR_LEAN_GATED != 0,
    );
    let clock = match (
        flags & RENOIR_LEAN_STAMP_NS != 0,
        flags & RENOIR_LEAN_STAMP_TICKS != 0,
    ) {
        (false, false) => LeanClock::None,
        (true, false) => LeanClock::Nanoseconds,
        (false, true) => LeanClock::Ticks,
        (true, true) => return RenoirErrorCode::InvalidParameter,
    };
    let opened = if clock.is_stamped() {
        LeanChannel::open_stamped(
            name,
            msg_size as usize,
            slot_count as usize,
            from_oldest,
            mode,
            clock,
        )
    } else {
        LeanChannel::open_with_mode(
            name,
            msg_size as usize,
            slot_count as usize,
            from_oldest,
            mode,
        )
    };
    match opened {
        Ok(mut channel) => {
            if flags & RENOIR_LEAN_CONSUMER != 0 {
                if let Err(e) = channel.register_consumer(from_oldest) {
                    return e.into();
                }
            }
            unsafe { *out = Box::into_raw(Box::new(channel)) as RenoirLeanHandle };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_memory_required(
    msg_size: u32,
    slot_count: u32,
    out_bytes: *mut usize,
) -> RenoirErrorCode {
    if out_bytes.is_null() || slot_count < 2 {
        return RenoirErrorCode::InvalidParameter;
    }
    let Some(slot_bytes) = crate::topic_rings::lean_slot_bytes(msg_size as usize) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let ring_bytes = slot_bytes * slot_count as usize;
    let slots = crate::topic_rings::LeanRing::slot_count_for(ring_bytes, msg_size as usize);
    unsafe {
        *out_bytes = crate::topic_rings::LEAN_HEADER_BYTES + slots * slot_bytes;
    }
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
pub extern "C" fn renoir_lean_publish_batch(
    h: RenoirLeanHandle,
    data: *const u8,
    len: usize,
    published: *mut usize,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if data.is_null() || published.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let src = unsafe { std::slice::from_raw_parts(data, len) };
    match h.publish_batch(src) {
        Ok(n) => {
            unsafe { *published = n };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_read_batch(
    h: RenoirLeanHandle,
    buffer: *mut u8,
    buffer_len: usize,
    count: *mut usize,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    if buffer.is_null() || count.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    let out = unsafe { std::slice::from_raw_parts_mut(buffer, buffer_len) };
    match h.read_batch(out) {
        Ok(n) => {
            unsafe { *count = n };
            RenoirErrorCode::Success
        }
        Err(e) => e.into(),
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_set_deadline(
    h: RenoirLeanHandle,
    deadline_ns: u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    h.set_deadline((deadline_ns != 0).then(|| Duration::from_nanos(deadline_ns)));
    RenoirErrorCode::Success
}

#[no_mangle]
pub extern "C" fn renoir_lean_wait_deadline(
    h: RenoirLeanHandle,
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
pub extern "C" fn renoir_lean_deadline_misses(h: RenoirLeanHandle) -> u64 {
    handle(h).map_or(0, |h| h.deadline_misses())
}

#[no_mangle]
pub extern "C" fn renoir_lean_wait_adaptive(
    h: RenoirLeanHandle,
    timeout_ns: u64,
) -> RenoirErrorCode {
    let Some(h) = handle(h) else {
        return RenoirErrorCode::InvalidParameter;
    };
    let timeout = (timeout_ns != u64::MAX).then(|| Duration::from_nanos(timeout_ns));
    if h.wait_adaptive(timeout) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::BufferEmpty
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
pub extern "C" fn renoir_lean_live_publishers(h: RenoirLeanHandle) -> usize {
    handle(h).map_or(0, |h| h.live_publishers())
}

#[no_mangle]
pub extern "C" fn renoir_lean_consumer_id(h: RenoirLeanHandle) -> i32 {
    handle(h)
        .and_then(|h| h.consumer_id())
        .map_or(-1, |id| id as i32)
}

#[no_mangle]
pub extern "C" fn renoir_lean_set_upstream(h: RenoirLeanHandle, upstream: u32) -> RenoirErrorCode {
    match handle(h) {
        Some(h) => match h.set_upstream(upstream as usize) {
            Ok(()) => RenoirErrorCode::Success,
            Err(e) => e.into(),
        },
        None => RenoirErrorCode::InvalidParameter,
    }
}

#[no_mangle]
pub extern "C" fn renoir_lean_close(h: RenoirLeanHandle) {
    if !h.is_null() {
        drop(unsafe { Box::from_raw(h as *mut LeanChannel) });
    }
}

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
