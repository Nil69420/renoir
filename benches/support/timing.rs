pub const WARMUP: usize = 20_000;
pub const SAMPLES: usize = 300_000;

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn ticks() -> u64 {
    use core::arch::x86_64::{_mm_lfence, _rdtsc};
    unsafe {
        _mm_lfence();
        let t = _rdtsc();
        _mm_lfence();
        t
    }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
pub fn ticks() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u64
}

pub fn ticks_per_ns() -> f64 {
    #[cfg(target_arch = "x86_64")]
    {
        let t0 = ticks();
        let w0 = std::time::Instant::now();
        while w0.elapsed().as_millis() < 200 {}
        let t1 = ticks();
        (t1 - t0) as f64 / w0.elapsed().as_nanos() as f64
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1.0
    }
}

pub fn samples_for(payload: usize) -> (usize, usize) {
    if payload <= 4096 {
        (WARMUP, SAMPLES)
    } else {
        (500, 20_000)
    }
}

pub fn pin(cpu: usize) {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

pub struct AlignedBuf {
    storage: Vec<u8>,
    offset: usize,
    len: usize,
}

impl std::ops::Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.storage[self.offset..self.offset + self.len]
    }
}

impl std::ops::DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.storage[self.offset..self.offset + self.len]
    }
}

pub fn aligned_buf(len: usize, fill: u8) -> AlignedBuf {
    let storage = vec![fill; len + 256];
    let offset = (128 - storage.as_ptr() as usize % 128) % 128;
    AlignedBuf {
        storage,
        offset,
        len,
    }
}

pub fn aligned_mem(total: usize) -> (std::ptr::NonNull<u8>, &'static mut [u64]) {
    let mem: &'static mut [u64] = Box::leak(vec![0u64; total / 8 + 1025].into_boxed_slice());
    let base = mem.as_mut_ptr() as usize;
    let aligned = (base + 4095) & !4095;
    (std::ptr::NonNull::new(aligned as *mut u8).unwrap(), mem)
}

pub fn report(name: &str, mut cycles: Vec<u64>, tpn: f64) {
    cycles.sort_unstable();
    let ns = |c: u64| c as f64 / tpn;
    let n = cycles.len();
    println!(
        "{:<34} RTT p50 {:6.0} ns  p99 {:6.0} ns  p99.9 {:6.0} ns  max {:8.0} ns   one-way ~p50 {:5.0} ns",
        name,
        ns(cycles[n / 2]),
        ns(cycles[n * 99 / 100]),
        ns(cycles[n * 999 / 1000]),
        ns(cycles[n - 1]),
        ns(cycles[n / 2]) / 2.0
    );
}
