use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use renoir::topic_rings::{lean_slot_bytes, LeanRead, LeanRing};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static TRACKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn track() {
    TRACKED.with(|t| t.set(true));
}

fn count() {
    if TRACKED.try_with(std::cell::Cell::get).unwrap_or(false) {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count();
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn publish_and_read_do_not_allocate() {
    track();
    let msg = 48;
    let ring_bytes = lean_slot_bytes(msg).unwrap() * 64;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring =
        Arc::new(unsafe { LeanRing::from_memory(ptr, mem.len() * 8, ring_bytes, msg).unwrap() });
    let mut publisher = ring.publisher();
    let mut cursor = ring.cursor_latest();
    let payload = [9u8; 48];
    let mut out = [0u8; 48];
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..100_000 {
        publisher.publish(&payload).unwrap();
        assert!(matches!(
            ring.try_read(&mut cursor, &mut out),
            Ok(LeanRead::Message)
        ));
        let _ = ring.pending(&cursor);
    }
    let after = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(after - before, 0);
}
