use crate::sensor::{self, RenoirImageFrameHeader, RenoirImuSample};

use super::types::RenoirErrorCode;

/// Total payload bytes for an image frame (header plus pixels); 0 if the
/// geometry or encoding is invalid. A `stride` of 0 means tightly packed.
#[no_mangle]
pub extern "C" fn renoir_image_frame_payload_size(
    width: u32,
    height: u32,
    stride: u32,
    encoding: u16,
) -> usize {
    sensor::frame_payload_size(width, height, stride, encoding).unwrap_or(0)
}

/// Initialise an image frame header in caller memory (intrinsics default to zero).
#[no_mangle]
pub extern "C" fn renoir_image_frame_header_init(
    header: *mut RenoirImageFrameHeader,
    width: u32,
    height: u32,
    stride: u32,
    encoding: u16,
    stamp_ns: u64,
    camera_id: u32,
    frame_seq: u64,
) -> RenoirErrorCode {
    if header.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    match sensor::init_header(
        width, height, stride, encoding, stamp_ns, camera_id, frame_seq,
    ) {
        Some(h) => {
            // SAFETY: header is non-null and the caller guarantees it points to writable
            // memory for one RenoirImageFrameHeader; unaligned write tolerates any placement.
            unsafe { std::ptr::write_unaligned(header, h) };
            RenoirErrorCode::Success
        }
        None => RenoirErrorCode::InvalidParameter,
    }
}

/// Check that a received payload holds a well-formed image frame.
#[no_mangle]
pub extern "C" fn renoir_image_frame_validate(payload: *const u8, len: usize) -> RenoirErrorCode {
    if payload.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    // SAFETY: caller guarantees payload points to len readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(payload, len) };
    if sensor::validate_frame(bytes) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::SerializationError
    }
}

/// Initialise an IMU sample in caller memory; gyro and accel are 3 doubles each.
#[no_mangle]
pub extern "C" fn renoir_imu_sample_init(
    sample: *mut RenoirImuSample,
    stamp_ns: u64,
    gyro: *const f64,
    accel: *const f64,
    imu_id: u32,
    seq: u64,
) -> RenoirErrorCode {
    if sample.is_null() || gyro.is_null() || accel.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    // SAFETY: gyro/accel are non-null and the caller guarantees 3 readable doubles each.
    let (g, a) = unsafe {
        (
            [*gyro, *gyro.add(1), *gyro.add(2)],
            [*accel, *accel.add(1), *accel.add(2)],
        )
    };
    // SAFETY: sample is non-null and points to writable memory for one RenoirImuSample.
    unsafe { std::ptr::write_unaligned(sample, sensor::init_sample(stamp_ns, g, a, imu_id, seq)) };
    RenoirErrorCode::Success
}

/// Check that a received payload holds a well-formed IMU sample.
#[no_mangle]
pub extern "C" fn renoir_imu_sample_validate(payload: *const u8, len: usize) -> RenoirErrorCode {
    if payload.is_null() {
        return RenoirErrorCode::InvalidParameter;
    }
    // SAFETY: caller guarantees payload points to len readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(payload, len) };
    if sensor::validate_sample(bytes) {
        RenoirErrorCode::Success
    } else {
        RenoirErrorCode::SerializationError
    }
}
