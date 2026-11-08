use super::{
    frame::{frame_slot_bytes, LEAN_MAX_FRAME_BYTES},
    inspect::inspect,
    lean_slot_bytes, LeanRing, LEAN_HEADER_BYTES, LEAN_MAX_SLOT_LINES,
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
