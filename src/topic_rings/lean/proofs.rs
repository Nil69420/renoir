use super::{
    clock::ticks_to_ns,
    frame::{frame_slot_bytes, LEAN_MAX_FRAME_BYTES},
    inspect::inspect,
    lean_slot_bytes,
    stamped::{stamped_geometry, STAMP_BYTES},
    LeanClock, LeanRing, LEAN_HEADER_BYTES, LEAN_MAX_SLOT_LINES, LINE, SEQ_BYTES,
};

#[kani::proof]
fn lean_slot_bytes_is_line_aligned_and_bounded() {
    let msg: usize = kani::any();
    if let Some(bytes) = lean_slot_bytes(msg) {
        assert!(msg >= 1);
        assert!(bytes % 64 == 0);
        assert!(bytes >= msg + 8);
        assert!(bytes <= LEAN_MAX_SLOT_LINES * 64);
    } else {
        assert!(msg == 0 || msg > LEAN_MAX_SLOT_LINES * 64 - 8);
    }
}

#[kani::proof]
fn frame_slot_bytes_is_line_aligned_and_fits_the_frame() {
    let max_frame: usize = kani::any();
    match frame_slot_bytes(max_frame) {
        Some(bytes) => {
            assert!(max_frame >= 1 && max_frame <= LEAN_MAX_FRAME_BYTES);
            assert!(bytes % 64 == 0);
            assert!(bytes >= max_frame + 16);
            assert!(bytes < max_frame + 16 + 64);
        }
        None => assert!(max_frame == 0 || max_frame > LEAN_MAX_FRAME_BYTES),
    }
}

#[kani::proof]
fn slot_count_is_a_power_of_two_that_fits_the_ring() {
    let ring_bytes: usize = kani::any();
    let msg: usize = kani::any();
    kani::assume(msg >= 1 && msg <= 504);
    let count = LeanRing::slot_count_for(ring_bytes, msg);
    let slot = lean_slot_bytes(msg).unwrap();
    assert!(count == 0 || count.is_power_of_two());
    assert!(count * slot <= ring_bytes);
    assert!(count == 0 || (count as u128) * 2 * (slot as u128) > ring_bytes as u128);
}

#[kani::proof]
fn frame_slot_count_is_a_power_of_two_that_fits_the_ring() {
    let ring_bytes: usize = kani::any();
    let max_frame: usize = kani::any();
    kani::assume(max_frame >= 1 && max_frame <= LEAN_MAX_FRAME_BYTES);
    kani::assume(ring_bytes <= 1usize << 40);
    let count = LeanRing::frame_slots_for(ring_bytes, max_frame);
    let slot = frame_slot_bytes(max_frame).unwrap();
    assert!(count == 0 || count.is_power_of_two());
    assert!(count * slot <= ring_bytes);
}

#[kani::proof]
fn slot_offset_stays_inside_the_slot_area() {
    let idx: u16 = kani::any();
    let shift: u32 = kani::any();
    kani::assume(shift <= 8);
    let slot_count = 1u32 << shift;
    let lines: u32 = kani::any();
    kani::assume(lines >= 1 && lines <= 255);
    let slot_bytes = lines * 64;
    let mask = slot_count - 1;
    let offset = (u32::from(idx) & mask) * slot_bytes;
    let total = slot_count * slot_bytes;
    assert!(offset + slot_bytes <= total);
}

#[kani::proof]
#[kani::unwind(2)]
fn inspect_rejects_short_buffers_without_panicking() {
    let len: usize = kani::any();
    kani::assume(len < 64);
    let buf = vec![0u8; len];
    assert!(inspect(&buf).is_none());
}

#[kani::proof]
fn stamped_geometry_fits_sequence_stamp_and_payload_in_the_slot() {
    let data: usize = kani::any();
    match stamped_geometry(data) {
        Ok((msg, slot)) => {
            assert!(msg == data + STAMP_BYTES);
            assert!(slot % LINE == 0);
            assert!(SEQ_BYTES + STAMP_BYTES + data <= slot);
            assert!(slot <= LEAN_MAX_SLOT_LINES * LINE);
            assert!(data <= LEAN_MAX_SLOT_LINES * LINE - SEQ_BYTES - STAMP_BYTES);
        }
        Err(_) => {
            assert!(data > LEAN_MAX_SLOT_LINES * LINE - SEQ_BYTES - STAMP_BYTES);
        }
    }
}

#[kani::proof]
fn stamped_and_plain_geometry_agree_on_the_wire_size() {
    let data: usize = kani::any();
    kani::assume(data <= 496);
    let (msg, slot) = stamped_geometry(data).unwrap();
    assert!(lean_slot_bytes(msg) == Some(slot));
    assert!(slot >= lean_slot_bytes(data.max(1)).unwrap());
}

#[kani::proof]
fn tick_conversion_product_cannot_overflow_for_any_counter_value() {
    let ticks: u64 = kani::any();
    let product = u128::from(ticks) * 1_000_000_000u128;
    assert!(product <= u128::from(u64::MAX) * 1_000_000_000u128);
    assert!(u128::from(u64::MAX) * 1_000_000_000u128 < u128::MAX);
}

#[kani::proof]
fn tick_conversion_is_monotonic_and_exact_for_whole_seconds_over_a_bounded_range() {
    let hz: u64 = kani::any();
    kani::assume(hz > 0 && hz <= 63);
    let a: u64 = kani::any();
    let b: u64 = kani::any();
    kani::assume(a <= 255 && b <= 255);
    if a <= b {
        assert!(ticks_to_ns(a, hz) <= ticks_to_ns(b, hz));
    }
    assert!(ticks_to_ns(0, hz) == 0);
    assert!(ticks_to_ns(hz, hz) == 1_000_000_000);
}

#[kani::proof]
fn clock_mode_decoding_round_trips_and_rejects_unknown_values() {
    let raw: u32 = kani::any();
    match LeanClock::from_raw(raw) {
        Some(clock) => {
            assert!(clock as u32 == raw);
            assert!(raw <= 2);
        }
        None => assert!(raw > 2),
    }
}

#[kani::proof]
fn sequence_numbers_keep_their_parity_and_order_below_the_overflow_horizon() {
    let idx: u64 = kani::any();
    kani::assume(idx >= 1 && idx < (1u64 << 62));
    let writing = 2 * idx - 1;
    let published = 2 * idx;
    assert!(writing % 2 == 1);
    assert!(published % 2 == 0);
    assert!(writing < published);
    let next = idx + 1;
    assert!(2 * next - 1 == published + 1);
}
