#![no_main]
use libfuzzer_sys::fuzz_target;
use renoir::topic_rings::{lean_slot_bytes, LeanMode, LeanRead, LeanRing};

const SLOTS: usize = 8;
const MSG: usize = 100;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    let mode = LeanMode::new(selector & 1 != 0, selector & 2 != 0);
    let slot = lean_slot_bytes(MSG).unwrap();
    let ring_bytes = slot * SLOTS;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut memory = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(memory.as_mut_ptr() as *mut u8).unwrap();
    let ring = match unsafe {
        LeanRing::from_memory_mode(ptr, memory.len() * 8, ring_bytes, MSG, mode)
    } {
        Ok(r) => std::sync::Arc::new(r),
        Err(_) => return,
    };
    let bytes: &mut [u8] =
        unsafe { std::slice::from_raw_parts_mut(memory.as_mut_ptr() as *mut u8, memory.len() * 8) };
    let region = 192usize;
    let n = body.len().min(bytes.len() - region);
    bytes[region..region + n].copy_from_slice(&body[..n]);
    let published = u64::from(selector >> 2) % 40;
    bytes[64..72].copy_from_slice(&published.to_ne_bytes());
    if mode.is_multi_producer() {
        bytes[136..144].copy_from_slice(&(published + u64::from(selector & 3)).to_ne_bytes());
    }

    let mut cursor = ring.cursor_oldest();
    let mut dst = [0u8; MSG];
    let mut batch = [0u8; MSG * 4];
    for _ in 0..16 {
        let _ = ring.pending(&cursor);
        match ring.try_read(&mut cursor, &mut dst) {
            Ok(LeanRead::Empty) | Err(_) => break,
            Ok(_) => {}
        }
    }
    let _ = ring.read_batch(&mut cursor, &mut batch);
    let _ = ring.skip_to_latest(&mut cursor, 3);
    if mode.is_gated() {
        if let Ok(mut gated) = ring.register_consumer(selector & 4 != 0) {
            for _ in 0..8 {
                match ring.try_read_gated(&mut gated, &mut dst) {
                    Ok(LeanRead::Empty) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = ring.read_batch_gated(&mut gated, &mut batch);
            ring.unregister_consumer(&mut gated);
        }
    }
    if !mode.is_multi_producer() {
        let mut publisher = ring.publisher();
        let _ = publisher.publish(&dst);
        let _ = publisher.publish_batch(&batch);
        let _ = ring.try_read(&mut cursor, &mut dst);
    }
});
