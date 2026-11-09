use renoir::topic_rings::{BroadcastTopicRing, MPMCTopicRing, SPSCTopicRing};

fn ring_bytes(magic: u64, capacity: usize, total: usize) -> Vec<u8> {
    let mut b = vec![0u8; total];
    b[0..8].copy_from_slice(&magic.to_ne_bytes());
    b[8..16].copy_from_slice(&(capacity as u64).to_ne_bytes());
    b
}

const SPSC_MAGIC: u64 = renoir::topic_rings::SPSC_RING_MAGIC;
const MPMC_MAGIC: u64 = renoir::topic_rings::MPMC_RING_MAGIC;

fn probe_all(bytes: &[u8]) {
    let _ = SPSCTopicRing::read_control_info(bytes);
    let _ = SPSCTopicRing::read_pending_header(bytes);
    let _ = MPMCTopicRing::read_control_info(bytes);
    let _ = MPMCTopicRing::read_pending_header(bytes);
    let _ = BroadcastTopicRing::read_control_info(bytes);
}

#[test]
fn test_zero_and_non_power_of_two_capacity_are_rejected() {
    for magic in [SPSC_MAGIC, MPMC_MAGIC] {
        for capacity in [0usize, 3, 100, 1000] {
            let b = ring_bytes(magic, capacity, 4096);
            assert!(SPSCTopicRing::read_control_info(&b).is_none() || magic == SPSC_MAGIC);
            probe_all(&b);
        }
    }
    assert!(SPSCTopicRing::read_control_info(&ring_bytes(SPSC_MAGIC, 0, 4096)).is_none());
    assert!(MPMCTopicRing::read_control_info(&ring_bytes(MPMC_MAGIC, 0, 4096)).is_none());
    assert!(SPSCTopicRing::read_control_info(&ring_bytes(SPSC_MAGIC, 100, 4096)).is_none());
}

#[test]
fn test_capacity_larger_than_the_mapping_is_rejected() {
    let b = ring_bytes(SPSC_MAGIC, 1 << 40, 4096);
    assert!(SPSCTopicRing::read_control_info(&b).is_none());
    let b = ring_bytes(MPMC_MAGIC, 1 << 40, 4096);
    assert!(MPMCTopicRing::read_control_info(&b).is_none());
}

#[test]
fn test_capacity_smaller_than_one_message_header_never_panics() {
    for capacity in [1usize, 2, 4, 8, 16, 32] {
        for (magic, spsc) in [(SPSC_MAGIC, true), (MPMC_MAGIC, false)] {
            let mut b = ring_bytes(magic, capacity, 4096);
            b[16..80].fill(0xAC);
            if spsc {
                assert!(SPSCTopicRing::read_pending_header(&b).is_none());
            } else {
                assert!(MPMCTopicRing::read_pending_header(&b).is_none());
            }
        }
    }
}

#[test]
fn test_misaligned_and_truncated_slices_never_panic() {
    let mut backing = vec![0u8; 4096 + 16];
    backing[8..16].copy_from_slice(&SPSC_MAGIC.to_ne_bytes());
    backing[16..24].copy_from_slice(&1024u64.to_ne_bytes());
    for offset in 0..8 {
        for len in [0usize, 1, 15, 16, 23, 24, 64, 4096] {
            probe_all(&backing[offset..(offset + len).min(backing.len())]);
        }
    }
}

#[test]
fn test_crash_input_found_by_fuzzing_does_not_panic() {
    let hex = "524f4e5243504d4d00000000000000000000 4dac acac acac acac acac acac acac acac 3000 acac acac 0100 ffff ffff acac ac68 acac acac acac 3000 acac acac 0100";
    let bytes: Vec<u8> = hex
        .split_whitespace()
        .collect::<String>()
        .as_bytes()
        .chunks(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect();
    probe_all(&bytes);
}
