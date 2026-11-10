use std::ptr::NonNull;
use std::sync::Arc;

use renoir::topic_rings::{lean_slot_bytes, LeanClock, LeanMode, LeanRead, LeanRing, STAMP_BYTES};

const WRITE_IDX: usize = 64;
const CLAIM: usize = 136;
const HOSTILE: [u64; 5] = [
    (1 << 62) + 1,
    1 << 63,
    u64::MAX - 1,
    u64::MAX,
    0x7FFF_FFFF_FFFF_FFFF,
];

fn ring(mode: LeanMode, stamped: bool) -> (Arc<LeanRing>, Vec<u64>) {
    let data = 16;
    let wire = if stamped { data + STAMP_BYTES } else { data };
    let ring_bytes = lean_slot_bytes(wire).unwrap() * 8;
    let total = LeanRing::required_memory_size(ring_bytes, wire);
    let mut mem = vec![0u64; total / 8 + 8];
    let ptr = NonNull::new(mem.as_mut_ptr().cast::<u8>()).unwrap();
    let ring = unsafe {
        if stamped {
            LeanRing::from_memory_stamped(
                ptr,
                mem.len() * 8,
                ring_bytes,
                data,
                mode,
                LeanClock::Nanoseconds,
            )
        } else {
            LeanRing::from_memory_mode(ptr, mem.len() * 8, ring_bytes, data, mode)
        }
        .unwrap()
    };
    (Arc::new(ring), mem)
}

fn poke(mem: &mut [u64], offset: usize, value: u64) {
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(mem.as_mut_ptr().cast::<u8>(), mem.len() * 8) };
    bytes[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
}

#[test]
fn a_hostile_write_index_never_panics_a_reader_or_a_single_producer() {
    for stamped in [false, true] {
        for hostile in HOSTILE {
            let (ring, mut mem) = ring(LeanMode::SingleProducer, stamped);
            poke(&mut mem, WRITE_IDX, hostile);
            let mut cursor = ring.cursor_oldest();
            let mut out = [0u8; 16];
            for _ in 0..4 {
                let _ = ring.pending(&cursor);
                if stamped {
                    let _ = ring.try_read_stamped(&mut cursor, &mut out);
                } else {
                    let _ = ring.try_read(&mut cursor, &mut out);
                }
            }
            let _ = ring.skip_to_latest(&mut cursor, 3);
            let _ = ring.cursor_latest();
            let mut publisher = ring.publisher();
            let _ = if stamped {
                publisher.publish_at(1, &out)
            } else {
                publisher.publish(&out)
            };
            let _ = publisher.publish_batch(&[0u8; 16 * 3]);
            assert!(matches!(
                ring.try_read(&mut ring.cursor_latest(), &mut out),
                Ok(LeanRead::Empty) | Ok(LeanRead::Message) | Err(_)
            ));
        }
    }
}

#[test]
fn a_hostile_claim_counter_is_refused_instead_of_overflowing() {
    for hostile in HOSTILE {
        for gated in [false, true] {
            let mode = LeanMode::new(true, gated);
            let (single, mut mem) = ring(mode, false);
            poke(&mut mem, CLAIM, hostile);
            let result = single.publisher().publish(&[1u8; 16]);
            assert!(result.is_err(), "publish {hostile:#x} gated={gated}");

            let (batch, mut mem) = ring(mode, false);
            poke(&mut mem, CLAIM, hostile);
            let result = batch.publisher().publish_batch(&[1u8; 16 * 3]);
            assert!(result.is_err(), "batch {hostile:#x} gated={gated}");
        }
    }
}
