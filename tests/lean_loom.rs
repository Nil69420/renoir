#![cfg(loom)]
use loom::sync::atomic::{fence, AtomicU64, Ordering};
use loom::sync::Arc;
use loom::thread;

struct Slot {
    seq: AtomicU64,
    a: AtomicU64,
    b: AtomicU64,
}

struct Ring {
    slots: [Slot; 2],
    write_idx: AtomicU64,
}

fn new_ring() -> Arc<Ring> {
    let slot = || Slot {
        seq: AtomicU64::new(0),
        a: AtomicU64::new(0),
        b: AtomicU64::new(0),
    };
    Arc::new(Ring {
        slots: [slot(), slot()],
        write_idx: AtomicU64::new(0),
    })
}

fn publish(r: &Ring, idx: u64) {
    let s = &r.slots[((idx - 1) & 1) as usize];
    s.seq.store(2 * idx - 1, Ordering::Relaxed);
    fence(Ordering::Release);
    s.a.store(idx, Ordering::Relaxed);
    s.b.store(idx, Ordering::Relaxed);
    s.seq.store(2 * idx, Ordering::Release);
    r.write_idx.store(idx, Ordering::Release);
}

fn try_read(r: &Ring, idx: u64) -> Option<(u64, u64)> {
    let s = &r.slots[((idx - 1) & 1) as usize];
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

#[test]
fn accepted_reads_are_never_torn() {
    loom::model(|| {
        let ring = new_ring();
        let w = ring.clone();
        let writer = thread::spawn(move || {
            for idx in 1..=3 {
                publish(&w, idx);
            }
        });
        for idx in 1..=3u64 {
            if let Some((a, b)) = try_read(&ring, idx) {
                assert_eq!(a, b);
                assert_eq!(a, idx);
            }
        }
        writer.join().unwrap();
    });
}

#[test]
fn published_index_implies_stable_slot() {
    loom::model(|| {
        let ring = new_ring();
        let w = ring.clone();
        let writer = thread::spawn(move || publish(&w, 1));
        let seen = ring.write_idx.load(Ordering::Acquire);
        if seen == 1 {
            assert_eq!(try_read(&ring, 1), Some((1, 1)));
        }
        writer.join().unwrap();
    });
}

#[test]
fn lapped_reader_detects_overwrite() {
    loom::model(|| {
        let ring = new_ring();
        let w = ring.clone();
        let writer = thread::spawn(move || {
            for idx in 1..=3 {
                publish(&w, idx);
            }
        });
        if let Some((a, b)) = try_read(&ring, 1) {
            assert_eq!((a, b), (1, 1));
        }
        writer.join().unwrap();
    });
}

struct MpRing {
    ring: Ring,
    claim: AtomicU64,
}

fn publish_mp(m: &MpRing) {
    let idx = m.claim.fetch_add(1, Ordering::AcqRel) + 1;
    let s = &m.ring.slots[((idx - 1) & 1) as usize];
    let previous = if idx > 2 { 2 * (idx - 2) } else { 0 };
    loop {
        if s.seq
            .compare_exchange(previous, 2 * idx - 1, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            break;
        }
        thread::yield_now();
    }
    fence(Ordering::Release);
    s.a.store(idx, Ordering::Relaxed);
    s.b.store(idx, Ordering::Relaxed);
    assert!(s
        .seq
        .compare_exchange(2 * idx - 1, 2 * idx, Ordering::Release, Ordering::Relaxed)
        .is_ok());
    m.ring.write_idx.fetch_max(idx, Ordering::Release);
}

#[test]
fn multi_producer_reads_are_never_torn() {
    loom::model(|| {
        let ring = new_ring();
        let m = Arc::new(MpRing {
            ring: Arc::try_unwrap(ring).ok().unwrap(),
            claim: AtomicU64::new(0),
        });
        let a = m.clone();
        let b = m.clone();
        let t1 = thread::spawn(move || {
            publish_mp(&a);
            publish_mp(&a);
        });
        let t2 = thread::spawn(move || publish_mp(&b));
        for idx in 1..=3u64 {
            if let Some((x, y)) = try_read(&m.ring, idx) {
                assert_eq!(x, y);
                assert_eq!(x, idx);
            }
        }
        t1.join().unwrap();
        t2.join().unwrap();
        for idx in 2..=3u64 {
            if let Some((x, y)) = try_read(&m.ring, idx) {
                assert_eq!((x, y), (idx, idx));
            }
        }
    });
}

struct Gated {
    ring: Ring,
    pos: AtomicU64,
}

fn publish_gated(g: &Gated, idx: u64) {
    loop {
        let pos = g.pos.load(Ordering::Acquire);
        if idx < pos + 2 {
            break;
        }
        thread::yield_now();
    }
    publish(&g.ring, idx);
}

#[test]
fn gated_reader_is_never_overwritten_mid_read() {
    loom::model(|| {
        let g = Arc::new(Gated {
            ring: Arc::try_unwrap(new_ring()).ok().unwrap(),
            pos: AtomicU64::new(1),
        });
        let w = g.clone();
        let writer = thread::spawn(move || {
            for idx in 1..=3 {
                publish_gated(&w, idx);
            }
        });
        for idx in 1..=3u64 {
            let s = &g.ring.slots[((idx - 1) & 1) as usize];
            loop {
                let s1 = s.seq.load(Ordering::Acquire);
                if s1 == 2 * idx {
                    let a = s.a.load(Ordering::Relaxed);
                    let b = s.b.load(Ordering::Relaxed);
                    fence(Ordering::Acquire);
                    assert_eq!(s.seq.load(Ordering::Relaxed), s1, "overwritten mid-read");
                    assert_eq!((a, b), (idx, idx));
                    g.pos.store(idx + 1, Ordering::Release);
                    break;
                }
                assert!(s1 < 2 * idx, "lapped despite gating");
                thread::yield_now();
            }
        }
        writer.join().unwrap();
    });
}
