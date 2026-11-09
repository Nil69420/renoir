#![no_main]
use libfuzzer_sys::fuzz_target;
use renoir::topic_rings::{
    inspect_lean_header, lean_slot_bytes, LeanClock, LeanMode, LeanRead, LeanRing, LEAN_RING_MAGIC,
    STAMP_BYTES,
};

const WORDS: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let selector = data[0];
    let size_pick = usize::from(data[1]) | usize::from(data[2] & 1) << 8;
    let body = &data[3..];
    let mode = LeanMode::new(selector & 1 != 0, selector & 2 != 0);
    let clock = match (selector >> 2) & 3 {
        0 => LeanClock::None,
        1 => LeanClock::Nanoseconds,
        _ => LeanClock::Ticks,
    };

    let mut memory = vec![0u64; WORDS];
    let bytes: &mut [u8] =
        unsafe { std::slice::from_raw_parts_mut(memory.as_mut_ptr().cast::<u8>(), WORDS * 8) };
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = body[i % body.len().max(1)];
    }
    bytes[..8].copy_from_slice(&LEAN_RING_MAGIC.to_ne_bytes());

    let _ = inspect_lean_header(bytes);
    let ptr = std::ptr::NonNull::new(memory.as_mut_ptr().cast::<u8>()).unwrap();
    let data_size = size_pick % 497 + 1;
    let wire = if clock.is_stamped() {
        data_size + STAMP_BYTES
    } else {
        data_size
    };
    let Some(slot) = lean_slot_bytes(wire) else {
        return;
    };
    let ring_bytes = slot * 4;
    if selector & 0x80 == 0 {
        bytes[8..16].copy_from_slice(&4u64.to_ne_bytes());
        bytes[24..32].copy_from_slice(&(slot as u64).to_ne_bytes());
        bytes[32..40].copy_from_slice(&(wire as u64).to_ne_bytes());
        bytes[40..44].copy_from_slice(&(mode as u32).to_ne_bytes());
        bytes[44..48].copy_from_slice(&(clock as u32).to_ne_bytes());
    }
    let attached = unsafe {
        if clock.is_stamped() {
            LeanRing::from_memory_stamped(ptr, WORDS * 8, ring_bytes, data_size, mode, clock)
        } else {
            LeanRing::from_memory_mode(ptr, WORDS * 8, ring_bytes, data_size, mode)
        }
    };
    let Ok(ring) = attached else {
        return;
    };
    let ring = std::sync::Arc::new(ring);
    let mut out = vec![0u8; data_size];
    let mut cursor = ring.cursor_oldest();
    for _ in 0..8 {
        let step = if clock.is_stamped() {
            ring.try_read_stamped(&mut cursor, &mut out)
                .map(|s| s.is_some())
        } else {
            ring.try_read(&mut cursor, &mut out)
                .map(|r| r == LeanRead::Message)
        };
        if !matches!(step, Ok(true)) {
            break;
        }
    }
    let _ = ring.pending(&cursor);
    let _ = ring.skip_to_latest(&mut cursor, 2);
    let _ = ring.ticks_to_ns(u64::from(selector) << 56);
    let _ = (ring.data_size(), ring.tick_hz(), ring.clock());
    if !mode.is_multi_producer() && !mode.is_gated() {
        let mut publisher = ring.publisher();
        let payload = vec![7u8; data_size];
        let _ = if clock.is_stamped() {
            publisher.publish_at(u64::MAX, &payload)
        } else {
            publisher.publish(&payload)
        };
        let _ = ring.try_read(&mut cursor, &mut out);
    }
});
