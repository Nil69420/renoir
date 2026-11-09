use shuttle::sync::atomic::{fence, AtomicU64, Ordering};
use shuttle::sync::Arc;
use shuttle::thread;

const SLOTS: u64 = 4;

struct Slot {
    seq: AtomicU64,
    a: AtomicU64,
    b: AtomicU64,
}

struct Ring {
    slots: Vec<Slot>,
    claim: AtomicU64,
    write_idx: AtomicU64,
}

fn new_ring() -> Arc<Ring> {
    Arc::new(Ring {
        slots: (0..SLOTS)
            .map(|_| Slot {
                seq: AtomicU64::new(0),
                a: AtomicU64::new(0),
                b: AtomicU64::new(0),
            })
            .collect(),
        claim: AtomicU64::new(0),
        write_idx: AtomicU64::new(0),
    })
}

fn publish(r: &Ring, producer: u64) {
    let idx = r.claim.fetch_add(1, Ordering::AcqRel) + 1;
    let s = &r.slots[((idx - 1) % SLOTS) as usize];
    let previous = if idx > SLOTS { 2 * (idx - SLOTS) } else { 0 };
    loop {
        match s
            .seq
            .compare_exchange(previous, 2 * idx - 1, Ordering::AcqRel, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(seen) => {
                assert!(seen < 2 * idx - 1, "slot lapped before write");
                thread::yield_now();
            }
        }
    }
    fence(Ordering::Release);
    let value = (producer << 32) | idx;
    s.a.store(value, Ordering::Relaxed);
    s.b.store(value, Ordering::Relaxed);
    assert!(s
        .seq
        .compare_exchange(2 * idx - 1, 2 * idx, Ordering::Release, Ordering::Relaxed)
        .is_ok());
    r.write_idx.fetch_max(idx, Ordering::Release);
}

fn try_read(r: &Ring, idx: u64) -> Option<(u64, u64)> {
    let s = &r.slots[((idx - 1) % SLOTS) as usize];
    let s1 = s.seq.load(Ordering::Acquire);
    if s1 != 2 * idx {
        return None;
    }
    let a = s.a.load(Ordering::Relaxed);
    let b = s.b.load(Ordering::Relaxed);
    fence(Ordering::Acquire);
    if s.seq.load(Ordering::Relaxed) == s1 {
        Some((a, b))
    } else {
        None
    }
}

fn scenario() {
    let ring = new_ring();
    let producers: Vec<_> = (1..=3u64)
        .map(|p| {
            let ring = ring.clone();
            thread::spawn(move || {
                for _ in 0..3 {
                    publish(&ring, p);
                }
            })
        })
        .collect();
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let ring = ring.clone();
            thread::spawn(move || {
                let mut seen = 0;
                for idx in 1..=9u64 {
                    if let Some((a, b)) = try_read(&ring, idx) {
                        assert_eq!(a, b, "torn read");
                        assert_eq!(a & 0xFFFF_FFFF, idx, "wrong index");
                        assert!((1..=3).contains(&(a >> 32)), "bad producer");
                        seen += 1;
                    }
                    thread::yield_now();
                }
                seen
            })
        })
        .collect();
    for p in producers {
        p.join().unwrap();
    }
    for r in readers {
        r.join().unwrap();
    }
    assert_eq!(ring.claim.load(Ordering::Relaxed), 9);
    assert_eq!(ring.write_idx.load(Ordering::Relaxed), 9);
}

#[test]
fn multi_producer_ring_is_never_torn_under_random_schedules() {
    shuttle::check_random(scenario, 20_000);
}

#[test]
fn multi_producer_ring_is_never_torn_under_pct_schedules() {
    shuttle::check_pct(scenario, 20_000, 3);
}
