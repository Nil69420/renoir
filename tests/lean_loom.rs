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
