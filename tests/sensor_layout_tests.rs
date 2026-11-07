use renoir::sensor::{
    frame_payload_size, init_header, init_sample, validate_frame, validate_sample,
    RenoirImageFrameHeader, RenoirImuSample, IMAGE_FRAME_HEADER_BYTES, IMU_SAMPLE_BYTES,
    RENOIR_ENCODING_BGR8, RENOIR_ENCODING_MONO16, RENOIR_ENCODING_MONO8,
};

fn bytes_of<T: Copy>(v: &T) -> Vec<u8> {
    let mut out = vec![0u8; std::mem::size_of::<T>()];
    unsafe {
        std::ptr::copy_nonoverlapping(v as *const T as *const u8, out.as_mut_ptr(), out.len())
    };
    out
}

fn frame(width: u32, height: u32, encoding: u16) -> Vec<u8> {
    let h = init_header(width, height, 0, encoding, 123, 1, 7).unwrap();
    let total = frame_payload_size(width, height, 0, encoding).unwrap();
    let mut buf = bytes_of(&h);
    buf.resize(total, 0xAB);
    buf
}

#[test]
fn test_layout_sizes_are_fixed() {
    assert_eq!(std::mem::size_of::<RenoirImageFrameHeader>(), 128);
    assert_eq!(IMAGE_FRAME_HEADER_BYTES, 128);
    assert_eq!(std::mem::size_of::<RenoirImuSample>(), 128);
    assert_eq!(IMU_SAMPLE_BYTES, 128);
}

#[test]
fn test_payload_size_for_camera_frame_and_encodings() {
    assert_eq!(
        frame_payload_size(1280, 720, 0, RENOIR_ENCODING_MONO8),
        Some(128 + 921_600)
    );
    assert_eq!(
        frame_payload_size(1280, 720, 0, RENOIR_ENCODING_MONO16),
        Some(128 + 1_843_200)
    );
    assert_eq!(
        frame_payload_size(640, 480, 2048, RENOIR_ENCODING_BGR8),
        Some(128 + 2048 * 480)
    );
}

#[test]
fn test_payload_size_rejects_bad_geometry() {
    assert_eq!(frame_payload_size(0, 720, 0, RENOIR_ENCODING_MONO8), None);
    assert_eq!(frame_payload_size(1280, 0, 0, RENOIR_ENCODING_MONO8), None);
    assert_eq!(
        frame_payload_size(1280, 720, 100, RENOIR_ENCODING_MONO8),
        None
    );
    assert_eq!(frame_payload_size(1280, 720, 0, 99), None);
    assert_eq!(
        frame_payload_size(u32::MAX, u32::MAX, 0, RENOIR_ENCODING_BGR8),
        None
    );
    assert_eq!(
        frame_payload_size(u32::MAX, u32::MAX, u32::MAX, RENOIR_ENCODING_MONO8),
        if usize::BITS == 64 {
            Some(128 + (u32::MAX as usize) * (u32::MAX as usize))
        } else {
            None
        }
    );
}

#[test]
fn test_valid_frame_round_trips() {
    let buf = frame(64, 48, RENOIR_ENCODING_MONO8);
    assert!(validate_frame(&buf));
    let h: RenoirImageFrameHeader =
        unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const RenoirImageFrameHeader) };
    assert_eq!((h.width, h.height, h.stride), (64, 48, 64));
    assert_eq!((h.stamp_ns, h.camera_id, h.frame_seq), (123, 1, 7));
    assert_eq!(h.pixel_offset as usize, IMAGE_FRAME_HEADER_BYTES);
}

#[test]
fn test_frame_validation_rejects_truncated_and_corrupt_payloads() {
    let good = frame(64, 48, RENOIR_ENCODING_MONO8);
    assert!(!validate_frame(&[]));
    assert!(!validate_frame(&good[..127]));
    assert!(!validate_frame(&good[..good.len() - 1]));

    let mut bad_magic = good.clone();
    bad_magic[0] ^= 0xFF;
    assert!(!validate_frame(&bad_magic));

    let mut bad_version = good.clone();
    bad_version[4] = 9;
    assert!(!validate_frame(&bad_version));

    let mut huge = good.clone();
    huge[24..28].copy_from_slice(&u32::MAX.to_ne_bytes());
    huge[28..32].copy_from_slice(&u32::MAX.to_ne_bytes());
    assert!(!validate_frame(&huge));

    let mut bad_offset = good.clone();
    bad_offset[120..124].copy_from_slice(&64u32.to_ne_bytes());
    assert!(!validate_frame(&bad_offset));
}

#[test]
fn test_frame_validation_survives_arbitrary_bytes() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for len in [0usize, 1, 127, 128, 129, 4096] {
        for _ in 0..200 {
            let mut buf = vec![0u8; len];
            for b in buf.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *b = seed as u8;
            }
            let _ = validate_frame(&buf);
            let _ = validate_sample(&buf);
        }
    }
}

#[test]
fn test_imu_sample_round_trips_and_rejects_garbage() {
    let s = init_sample(42, [0.1, 0.2, 0.3], [0.0, 0.0, 9.81], 2, 5);
    let buf = bytes_of(&s);
    assert!(validate_sample(&buf));
    assert!(!validate_sample(&buf[..127]));
    let mut bad = buf.clone();
    bad[1] ^= 0xFF;
    assert!(!validate_sample(&bad));
    let back: RenoirImuSample =
        unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const RenoirImuSample) };
    assert_eq!(back.stamp_ns, 42);
    assert_eq!(back.gyro, [0.1, 0.2, 0.3]);
    assert_eq!(back.accel[2], 9.81);
    assert_eq!(back.seq, 5);
}
