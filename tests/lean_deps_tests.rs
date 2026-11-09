use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use renoir::topic_rings::{lean_slot_bytes, LeanMode, LeanRead, LeanRing};

const MSG: usize = 16;

fn make(slots: usize) -> (Arc<LeanRing>, Vec<u64>) {
    let ring_bytes = lean_slot_bytes(MSG).unwrap() * slots;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr().cast::<u8>()).unwrap();
    let ring = unsafe {
        LeanRing::from_memory_mode(
            ptr,
            mem.len() * 8,
            ring_bytes,
            MSG,
            LeanMode::SingleProducerGated,
        )
        .unwrap()
    };
    (Arc::new(ring), mem)
}

fn msg(n: u64) -> [u8; MSG] {
    let mut b = [0u8; MSG];
    b[..8].copy_from_slice(&n.to_le_bytes());
    b[8..].copy_from_slice(&n.to_le_bytes());
    b
}

fn value(b: &[u8; MSG]) -> u64 {
    let lo = u64::from_le_bytes(b[..8].try_into().unwrap());
    assert_eq!(lo, u64::from_le_bytes(b[8..].try_into().unwrap()), "torn");
    lo
}

#[test]
fn downstream_only_sees_what_upstream_has_consumed() {
    let (ring, _mem) = make(16);
    let mut a = ring.register_consumer(true).unwrap();
    let mut b = ring.register_consumer(true).unwrap();
    ring.set_upstream(&mut b, a.consumer_id().unwrap()).unwrap();
    let mut publisher = ring.publisher();
    let mut out = [0u8; MSG];
    for n in 1..=5 {
        publisher.publish(&msg(n)).unwrap();
    }
    assert_eq!(ring.pending_gated(&b), 0);
    assert_eq!(
        ring.try_read_gated(&mut b, &mut out).unwrap(),
        LeanRead::Empty
    );
    for n in 1..=3 {
        assert_eq!(
            ring.try_read_gated(&mut a, &mut out).unwrap(),
            LeanRead::Message
        );
        assert_eq!(value(&out), n);
    }
    assert_eq!(ring.pending_gated(&b), 3);
    for n in 1..=3 {
        assert_eq!(
            ring.try_read_gated(&mut b, &mut out).unwrap(),
            LeanRead::Message
        );
        assert_eq!(value(&out), n);
    }
    assert_eq!(
        ring.try_read_gated(&mut b, &mut out).unwrap(),
        LeanRead::Empty
    );
    let mut batch = [0u8; MSG * 8];
    assert_eq!(ring.read_batch_gated(&mut b, &mut batch).unwrap(), 0);
    assert_eq!(ring.read_batch_gated(&mut a, &mut batch).unwrap(), 2);
    assert_eq!(ring.read_batch_gated(&mut b, &mut batch).unwrap(), 2);
}

#[test]
fn dependency_validation_and_release() {
    let (ring, _mem) = make(8);
    let mut a = ring.register_consumer(true).unwrap();
    let mut b = ring.register_consumer(true).unwrap();
    let mut c = ring.register_consumer(true).unwrap();
    let (ia, ib, ic) = (
        a.consumer_id().unwrap(),
        b.consumer_id().unwrap(),
        c.consumer_id().unwrap(),
    );
    assert!(ring.set_upstream(&mut a, ia).is_err());
    assert!(ring.set_upstream(&mut a, 7).is_err());
    ring.set_upstream(&mut b, ia).unwrap();
    ring.set_upstream(&mut c, ib).unwrap();
    assert!(ring.set_upstream(&mut a, ic).is_err());
    assert!(ring.set_upstream(&mut a, ib).is_err());

    let mut publisher = ring.publisher();
    publisher.publish(&msg(1)).unwrap();
    let mut out = [0u8; MSG];
    assert_eq!(
        ring.try_read_gated(&mut c, &mut out).unwrap(),
        LeanRead::Empty
    );
    ring.unregister_consumer(&mut a);
    assert_eq!(
        ring.try_read_gated(&mut b, &mut out).unwrap(),
        LeanRead::Message
    );
    assert!(!b.has_upstream());
    assert_eq!(
        ring.try_read_gated(&mut c, &mut out).unwrap(),
        LeanRead::Message
    );
}

#[test]
fn pipeline_of_three_stages_delivers_every_message_once_and_in_order() {
    let (ring, _mem) = make(32);
    let total = if cfg!(miri) { 50u64 } else { 200_000 };
    let mut a = ring.register_consumer(true).unwrap();
    let mut b = ring.register_consumer(true).unwrap();
    let mut c = ring.register_consumer(true).unwrap();
    ring.set_upstream(&mut b, a.consumer_id().unwrap()).unwrap();
    ring.set_upstream(&mut c, b.consumer_id().unwrap()).unwrap();
    let progress = [
        std::sync::atomic::AtomicU64::new(0),
        std::sync::atomic::AtomicU64::new(0),
        std::sync::atomic::AtomicU64::new(0),
    ];
    thread::scope(|scope| {
        let ring_p = &ring;
        scope.spawn(move || {
            let mut publisher = ring_p.publisher();
            for n in 1..=total {
                while publisher.publish(&msg(n)).is_err() {
                    std::hint::spin_loop();
                }
            }
        });
        for (stage, cursor) in [(0usize, &mut a), (1, &mut b), (2, &mut c)] {
            let ring = &ring;
            let progress = &progress;
            scope.spawn(move || {
                let mut out = [0u8; MSG];
                let mut expect = 1u64;
                let deadline = Instant::now() + Duration::from_secs(120);
                while expect <= total {
                    assert!(
                        Instant::now() < deadline,
                        "stage {stage} starved at {expect}"
                    );
                    if ring.try_read_gated(cursor, &mut out).unwrap() == LeanRead::Message {
                        assert_eq!(value(&out), expect, "stage {stage} order");
                        progress[stage].store(expect, std::sync::atomic::Ordering::Release);
                        expect += 1;
                    }
                }
            });
        }
    });
}
