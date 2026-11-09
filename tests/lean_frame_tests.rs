use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use renoir::topic_rings::{LeanMode, LeanRing};

fn make(max_frame: usize, slots: usize, mode: LeanMode) -> (Arc<LeanRing>, Vec<u64>) {
    let slot = renoir::topic_rings::frame_slot_bytes(max_frame).unwrap();
    let ring_bytes = slot * slots;
    let total = LeanRing::required_frame_memory(ring_bytes, max_frame);
    let mut mem = vec![0u64; total / 8 + 8];
    let base = (mem.as_mut_ptr() as usize + 63) & !63;
    let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
    let ring =
        unsafe { LeanRing::from_frame_memory(ptr, total, ring_bytes, max_frame, mode).unwrap() };
    (Arc::new(ring), mem)
}

struct StopOnDrop<'a>(&'a AtomicBool);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn fill(buf: &mut [u8], tag: u8) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = tag.wrapping_add(i as u8);
    }
}

fn check(buf: &[u8], tag: u8) {
    for (i, b) in buf.iter().enumerate() {
        assert_eq!(*b, tag.wrapping_add(i as u8), "byte {i} of {}", buf.len());
    }
}

#[test]
fn frames_have_dynamic_length_and_arrive_in_order() {
    let (ring, _mem) = make(1000, 8, LeanMode::SingleProducerGated);
    let mut consumer = ring.register_consumer(false).unwrap();
    let mut publisher = ring.publisher();
    let sizes = [1usize, 17, 64, 999, 1000, 3, 512];
    for (n, size) in sizes.iter().enumerate() {
        let mut loan = publisher.loan_frame(*size).unwrap();
        fill(loan.bytes_mut(), n as u8);
        loan.commit().unwrap();
        let view = ring.view_frame(&mut consumer).unwrap().expect("frame");
        assert_eq!(view.len(), *size);
        check(view.bytes(), n as u8);
        assert!(view.is_intact());
    }
    assert!(ring.view_frame(&mut consumer).unwrap().is_none());
    assert!(publisher.loan_frame(0).is_err());
    assert!(publisher.loan_frame(1001).is_err());
}

#[test]
fn view_points_into_the_ring_memory_and_a_held_view_blocks_the_producer() {
    let (ring, mem) = make(256, 4, LeanMode::SingleProducerGated);
    let lo = mem.as_ptr() as usize;
    let hi = lo + mem.len() * 8;
    let mut consumer = ring.register_consumer(false).unwrap();
    let mut publisher = ring.publisher();
    for n in 0..4u8 {
        let mut loan = publisher.loan_frame(200).unwrap();
        fill(loan.bytes_mut(), n);
        loan.commit().unwrap();
    }
    assert!(publisher.loan_frame(10).is_err());
    {
        let view = ring.view_frame(&mut consumer).unwrap().unwrap();
        let p = view.bytes().as_ptr() as usize;
        assert!(
            p >= lo && p + view.len() <= hi,
            "view must be zero-copy into the mapping"
        );
        check(view.bytes(), 0);
        assert!(
            publisher.loan_frame(10).is_err(),
            "held view keeps its slot"
        );
    }
    let mut loan = publisher.loan_frame(10).unwrap();
    fill(loan.bytes_mut(), 9);
    loan.commit().unwrap();
    for n in 1..4u8 {
        let view = ring.view_frame(&mut consumer).unwrap().unwrap();
        check(view.bytes(), n);
    }
    let view = ring.view_frame(&mut consumer).unwrap().unwrap();
    assert_eq!(view.len(), 10);
    check(view.bytes(), 9);
}

#[test]
fn dropped_loans_publish_nothing_and_do_not_wedge_either_mode() {
    for mode in [LeanMode::SingleProducerGated, LeanMode::MultiProducerGated] {
        let (ring, _mem) = make(128, 4, mode);
        let mut consumer = ring.register_consumer(false).unwrap();
        let mut publisher = ring.publisher();
        for round in 0..20u8 {
            drop(publisher.loan_frame(50).unwrap());
            let mut loan = publisher.loan_frame(30).unwrap();
            fill(loan.bytes_mut(), round);
            loan.commit().unwrap();
            let view = ring.view_frame(&mut consumer).unwrap().expect("frame");
            assert_eq!(view.len(), 30, "{mode:?} round {round}");
            check(view.bytes(), round);
            drop(view);
            assert!(ring.view_frame(&mut consumer).unwrap().is_none());
        }
    }
}

#[test]
fn large_frames_round_trip() {
    let (ring, _mem) = make(921_600, 4, LeanMode::SingleProducerGated);
    let mut consumer = ring.register_consumer(false).unwrap();
    let mut publisher = ring.publisher();
    for n in 0..6u8 {
        let mut loan = publisher.loan_frame(921_600).unwrap();
        fill(loan.bytes_mut(), n);
        loan.commit().unwrap();
        let view = ring.view_frame(&mut consumer).unwrap().unwrap();
        assert_eq!(view.len(), 921_600);
        check(view.bytes(), n);
    }
}

#[test]
fn concurrent_producers_and_a_consumer_never_see_a_torn_frame() {
    let producers = 3u64;
    let per = if cfg!(miri) { 20u64 } else { 3000 };
    let (ring, _mem) = make(700, 8, LeanMode::MultiProducerGated);
    let mut consumer = ring.register_consumer(true).unwrap();
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        let _guard = StopOnDrop(&stop);
        for p in 0..producers {
            let ring = &ring;
            let stop = &stop;
            scope.spawn(move || {
                let mut publisher = ring.publisher();
                for n in 1..=per {
                    let size = 1 + ((p * 131 + n * 17) % 700) as usize;
                    loop {
                        match publisher.loan_frame(size) {
                            Ok(mut loan) => {
                                let buf = loan.bytes_mut();
                                buf[0] = p as u8;
                                let tag = (n % 251) as u8;
                                fill(&mut buf[1..], tag);
                                loan.commit().unwrap();
                                break;
                            }
                            Err(_) => {
                                if stop.load(Ordering::Relaxed) {
                                    return;
                                }
                                std::hint::spin_loop();
                            }
                        }
                    }
                }
            });
        }
        let mut last = [0u64; 3];
        let mut total = 0u64;
        let deadline = Instant::now() + Duration::from_secs(120);
        while total < producers * per {
            assert!(Instant::now() < deadline, "starved at {total}");
            if let Some(view) = ring.view_frame(&mut consumer).unwrap() {
                let bytes = view.bytes();
                let p = bytes[0] as usize;
                last[p] += 1;
                let n = last[p];
                let expected = 1 + ((p as u64 * 131 + n * 17) % 700) as usize;
                assert_eq!(bytes.len(), expected, "producer {p} frame {n} length");
                check(&bytes[1..], (n % 251) as u8);
                total += 1;
            }
        }
    });
    assert_eq!(consumer.lost(), 0);
}

#[test]
fn frames_older_than_the_lifespan_are_dropped_and_fresh_ones_kept() {
    use renoir::topic_rings::FrameChannel;
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("RENOIR_SHM_DIR", dir.path());
    let name = format!("frame_life_{}", std::process::id());
    let mut producer = FrameChannel::open(&name, 256, 8, false, false, false).unwrap();
    let mut consumer = FrameChannel::open(&name, 256, 8, false, true, false).unwrap();
    for tag in 0..3u8 {
        let mut loan = producer.loan(32).unwrap();
        loan.bytes_mut().fill(tag);
        loan.commit().unwrap();
    }
    thread::sleep(Duration::from_millis(60));
    for tag in 10..12u8 {
        let mut loan = producer.loan(32).unwrap();
        loan.bytes_mut().fill(tag);
        loan.commit().unwrap();
    }
    {
        let view = consumer
            .view_fresh(Duration::from_millis(30))
            .unwrap()
            .unwrap();
        assert_eq!(
            view.bytes()[0],
            10,
            "the three stale frames must be skipped"
        );
        assert!(view.age() < Duration::from_millis(60));
    }
    assert_eq!(consumer.stale_dropped(), 3);
    let view = consumer
        .view_fresh(Duration::from_millis(30))
        .unwrap()
        .unwrap();
    assert_eq!(view.bytes()[0], 11);
    drop(view);
    assert!(consumer
        .view_fresh(Duration::from_millis(30))
        .unwrap()
        .is_none());
    thread::sleep(Duration::from_millis(50));
    let mut loan = producer.loan(8).unwrap();
    loan.bytes_mut().fill(1);
    loan.commit().unwrap();
    assert!(consumer
        .view_fresh(Duration::from_secs(60))
        .unwrap()
        .is_some());
}
