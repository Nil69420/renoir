use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    ptr::NonNull,
    sync::{Arc, Barrier},
    thread,
};

use renoir::{
    topic::TopicStats,
    topic_rings::{mpmc::MPMCTopicRing, spsc::SPSCTopicRing},
};

const THREADS: usize = 8;
const ROUNDS: usize = if cfg!(miri) { 4 } else { 300 };
const CAPACITY: usize = 1024;

struct Mem(NonNull<u8>, Layout);

unsafe impl Send for Mem {}
unsafe impl Sync for Mem {}

impl Mem {
    fn new(size: usize) -> Self {
        let layout = Layout::from_size_align(size, 64).unwrap();
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) }).unwrap();
        Self(ptr, layout)
    }
}

impl Drop for Mem {
    fn drop(&mut self) {
        unsafe { dealloc(self.0.as_ptr(), self.1) }
    }
}

#[test]
fn spsc_concurrent_first_open_agrees_on_one_header() {
    let size = SPSCTopicRing::required_memory_size(CAPACITY);
    for _ in 0..ROUNDS {
        let mem = Arc::new(Mem::new(size));
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let mem = mem.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let ring = unsafe {
                        SPSCTopicRing::from_memory(
                            mem.0,
                            size,
                            CAPACITY,
                            Arc::new(TopicStats::default()),
                        )
                    };
                    ring.map(|r| r.capacity())
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap().unwrap(), CAPACITY);
        }
    }
}

#[test]
fn mpmc_concurrent_first_open_agrees_on_one_header() {
    let size = MPMCTopicRing::required_memory_size(CAPACITY);
    for _ in 0..ROUNDS {
        let mem = Arc::new(Mem::new(size));
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let mem = mem.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let ring = unsafe {
                        MPMCTopicRing::from_memory(
                            mem.0,
                            size,
                            CAPACITY,
                            Arc::new(TopicStats::default()),
                        )
                    };
                    ring.map(|r| r.capacity())
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap().unwrap(), CAPACITY);
        }
    }
}

const INITIALIZING_SENTINEL: u64 = 0x4942_4E49_5F42_5254;

#[test]
fn ring_left_half_initialised_by_a_dead_opener_is_taken_over() {
    let size = SPSCTopicRing::required_memory_size(CAPACITY);
    let mem = Mem::new(size);
    unsafe { (mem.0.as_ptr() as *mut u64).write(INITIALIZING_SENTINEL) };
    let started = std::time::Instant::now();
    let ring = unsafe {
        SPSCTopicRing::from_memory(mem.0, size, CAPACITY, Arc::new(TopicStats::default()))
    }
    .unwrap();
    assert_eq!(ring.capacity(), CAPACITY);
    assert!(started.elapsed() < std::time::Duration::from_secs(30));

    let size = MPMCTopicRing::required_memory_size(CAPACITY);
    let mem = Mem::new(size);
    unsafe { (mem.0.as_ptr() as *mut u64).write(INITIALIZING_SENTINEL) };
    let ring = unsafe {
        MPMCTopicRing::from_memory(mem.0, size, CAPACITY, Arc::new(TopicStats::default()))
    }
    .unwrap();
    assert_eq!(ring.capacity(), CAPACITY);
}
