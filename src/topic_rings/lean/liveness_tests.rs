use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::multi::fill_slot_multi;
use super::{lean_slot_bytes, LeanMode, LeanRead, LeanRing};

const MSG: usize = 16;

fn make() -> (std::sync::Arc<LeanRing>, Vec<u64>) {
    let ring_bytes = lean_slot_bytes(MSG).unwrap() * 8;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr().cast::<u8>()).unwrap();
    let ring = unsafe {
        LeanRing::from_memory_mode(
            ptr,
            mem.len() * 8,
            ring_bytes,
            MSG,
            LeanMode::MultiProducerGated,
        )
        .unwrap()
    };
    ring.set_gate_timeout(Duration::from_millis(20));
    (std::sync::Arc::new(ring), mem)
}

fn poll_for(
    ring: &LeanRing,
    cursor: &mut super::LeanGatedCursor,
    window: Duration,
) -> Option<[u8; MSG]> {
    let end = Instant::now() + window;
    let mut out = [0u8; MSG];
    while Instant::now() < end {
        for _ in 0..200 {
            if ring.try_read_gated(cursor, &mut out).unwrap() == LeanRead::Message {
                return Some(out);
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

fn reaped_pid() -> u32 {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[test]
#[cfg_attr(miri, ignore)]
fn alive_claimer_is_waited_for_past_every_timeout() {
    let (ring, _mem) = make();
    let mut cursor = ring.register_consumer(true).unwrap();
    let mut slow = ring.publisher();
    let mut fast = ring.publisher();
    let idx = slow.claim_index().unwrap();
    assert_eq!(idx, 1);
    fast.publish(&[2u8; MSG]).unwrap();

    assert!(poll_for(&ring, &mut cursor, Duration::from_millis(300)).is_none());
    assert_eq!(cursor.lost(), 0);

    fill_slot_multi(&ring, idx, &[1u8; MSG]).unwrap();
    slow.end_claim();
    ring.write_idx().fetch_max(idx, Ordering::Release);

    assert_eq!(
        poll_for(&ring, &mut cursor, Duration::from_secs(1)),
        Some([1u8; MSG])
    );
    assert_eq!(
        poll_for(&ring, &mut cursor, Duration::from_secs(1)),
        Some([2u8; MSG])
    );
    assert_eq!(cursor.lost(), 0);
}

#[test]
#[cfg_attr(miri, ignore)]
fn dead_claimer_is_skipped_promptly() {
    let (ring, _mem) = make();
    let mut cursor = ring.register_consumer(true).unwrap();
    let mut doomed = ring.publisher();
    let mut fast = ring.publisher();
    let idx = doomed.claim_index().unwrap();
    assert_eq!(idx, 1);
    fast.publish(&[2u8; MSG]).unwrap();
    let handle = doomed.producer.unwrap();
    ring.producer_entry(handle.entry)
        .pid
        .store(reaped_pid(), Ordering::Release);

    assert_eq!(
        poll_for(&ring, &mut cursor, Duration::from_millis(500)),
        Some([2u8; MSG])
    );
    assert_eq!(cursor.lost(), 1);
}

#[test]
#[cfg_attr(miri, ignore)]
fn producer_table_entries_are_reused_after_drop() {
    let (ring, _mem) = make();
    for _ in 0..(super::LEAN_MAX_PRODUCERS * 3) {
        let publisher = ring.publisher();
        assert!(publisher.producer.is_some());
    }
    let held: Vec<_> = (0..super::LEAN_MAX_PRODUCERS)
        .map(|_| ring.publisher())
        .collect();
    assert!(held.iter().all(|p| p.producer.is_some()));
    assert!(ring.publisher().producer.is_none());
}
