use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{frame_slot_bytes, LeanMode, LeanRing};

fn make() -> (std::sync::Arc<LeanRing>, Vec<u64>) {
    let max_frame = 128;
    let slot = frame_slot_bytes(max_frame).unwrap();
    let ring_bytes = slot * 4;
    let total = LeanRing::required_frame_memory(ring_bytes, max_frame);
    let mut mem = vec![0u64; total / 8 + 8];
    let base = (mem.as_mut_ptr() as usize + 63) & !63;
    let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
    let ring = unsafe {
        LeanRing::from_frame_memory(
            ptr,
            total,
            ring_bytes,
            max_frame,
            LeanMode::SingleProducerGated,
        )
        .unwrap()
    };
    ring.set_gate_timeout(Duration::from_millis(10));
    (std::sync::Arc::new(ring), mem)
}

fn reaped_pid() -> u32 {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[test]
#[cfg_attr(miri, ignore)]
fn live_holder_blocks_the_producer_and_dead_holder_is_reclaimed() {
    let (ring, _mem) = make();
    let mut consumer = ring.register_consumer(false).unwrap();
    let mut publisher = ring.publisher();
    for n in 0..4u8 {
        let mut loan = publisher.loan_frame(16).unwrap();
        loan.bytes_mut().fill(n);
        loan.commit().unwrap();
    }
    let id = consumer.consumer_id().unwrap();
    let view = ring.view_frame(&mut consumer).unwrap().unwrap();
    assert!(publisher.loan_frame(16).is_err());
    std::thread::sleep(Duration::from_millis(60));
    assert!(
        publisher.loan_frame(16).is_err(),
        "a live holder must not be evicted by the timer"
    );
    assert!(view.is_intact());

    ring.gate_entry(id)
        .pid
        .store(reaped_pid(), Ordering::Release);
    let mut reclaimed = false;
    for _ in 0..50 {
        if publisher.loan_frame(16).is_ok() {
            reclaimed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(12));
    }
    assert!(reclaimed, "a dead holder must be evicted");
    assert!(!view.is_intact());
}
