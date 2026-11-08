#![no_main]
use libfuzzer_sys::fuzz_target;
use renoir::topic_rings::{frame_slot_bytes, LeanMode, LeanRing};

const MAX_FRAME: usize = 200;
const SLOTS: usize = 4;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    let mode = if selector & 1 == 0 {
        LeanMode::SingleProducerGated
    } else {
        LeanMode::MultiProducerGated
    };
    let slot = frame_slot_bytes(MAX_FRAME).unwrap();
    let ring_bytes = slot * SLOTS;
    let total = LeanRing::required_frame_memory(ring_bytes, MAX_FRAME);
    let mut memory = vec![0u64; total / 8 + 8];
    let base = (memory.as_mut_ptr() as usize + 63) & !63;
    let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
    let ring = match unsafe { LeanRing::from_frame_memory(ptr, total, ring_bytes, MAX_FRAME, mode) } {
        Ok(r) => std::sync::Arc::new(r),
        Err(_) => return,
    };
    let bytes: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(base as *mut u8, total) };
    let region = 192usize;
    let n = body.len().min(total - region);
    bytes[region..region + n].copy_from_slice(&body[..n]);
    let published = u64::from(selector >> 2) % 12;
    bytes[64..72].copy_from_slice(&published.to_ne_bytes());
    if mode.is_multi_producer() {
        bytes[136..144].copy_from_slice(&(published + u64::from((selector >> 1) & 1)).to_ne_bytes());
    }
    let header = renoir::topic_rings::LEAN_HEADER_BYTES;
    for idx in 1..=published.min(SLOTS as u64) {
        let at = header + ((idx as usize) % SLOTS) * slot;
        if selector & 0x40 == 0 {
            bytes[at..at + 8].copy_from_slice(&(2 * idx).to_ne_bytes());
        }
    }

    if let Ok(mut cursor) = ring.register_consumer(true) {
        for _ in 0..6 {
            match ring.view_frame(&mut cursor) {
                Ok(Some(view)) => {
                    let sum: u8 = view.bytes().iter().fold(0, |a, b| a ^ b);
                    std::hint::black_box(sum);
                    let _ = view.is_intact();
                }
                Ok(None) | Err(_) => break,
            }
        }
        let mut publisher = ring.publisher();
        for len in [1usize, 17, MAX_FRAME] {
            if let Ok(mut loan) = publisher.loan_frame(len) {
                loan.bytes_mut().fill(0xA5);
                if selector & 0x80 == 0 {
                    let _ = loan.commit();
                }
            }
        }
        ring.unregister_consumer(&mut cursor);
    }
});
