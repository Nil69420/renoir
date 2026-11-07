#![no_main]
use libfuzzer_sys::fuzz_target;
use renoir::sensor::{frame_payload_size, validate_frame, validate_sample};

fuzz_target!(|data: &[u8]| {
    let _ = validate_frame(data);
    let _ = validate_sample(data);
    if data.len() >= 14 {
        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let stride = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let enc = u16::from_le_bytes([data[12], data[13]]);
        let _ = frame_payload_size(w, h, stride, enc);
    }
});
