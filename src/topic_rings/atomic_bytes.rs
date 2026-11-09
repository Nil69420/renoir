use std::sync::atomic::{AtomicU64, Ordering};

pub(super) unsafe fn atomic_store_bytes(dst: *mut u8, src: *const u8, len: usize) {
    let words = len / 8;
    for i in 0..words {
        let w = std::ptr::read_unaligned(src.add(i * 8) as *const u64);
        (*(dst.add(i * 8) as *const AtomicU64)).store(w, Ordering::Relaxed);
    }
    let rem = len % 8;
    if rem != 0 {
        let mut tail = [0u8; 8];
        std::ptr::copy_nonoverlapping(src.add(words * 8), tail.as_mut_ptr(), rem);
        (*(dst.add(words * 8) as *const AtomicU64))
            .store(u64::from_ne_bytes(tail), Ordering::Relaxed);
    }
}

pub(super) unsafe fn atomic_load_bytes(src: *const u8, dst: *mut u8, len: usize) {
    let words = len / 8;
    for i in 0..words {
        let w = (*(src.add(i * 8) as *const AtomicU64)).load(Ordering::Relaxed);
        std::ptr::write_unaligned(dst.add(i * 8) as *mut u64, w);
    }
    let rem = len % 8;
    if rem != 0 {
        let tail = (*(src.add(words * 8) as *const AtomicU64))
            .load(Ordering::Relaxed)
            .to_ne_bytes();
        std::ptr::copy_nonoverlapping(tail.as_ptr(), dst.add(words * 8), rem);
    }
}
