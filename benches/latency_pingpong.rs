use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::thread;

use renoir::topic::{MessageHeader, TopicStats, FORMAT_TYPE_INLINE};
use renoir::topic_rings::{BroadcastTopicRing, ReadInto};

const WARMUP: usize = 20_000;
const SAMPLES: usize = 300_000;

#[repr(align(64))]
struct Line(AtomicU64);

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn ticks() -> u64 {
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
fn ticks() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u64
}

fn ticks_per_ns() -> f64 {
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

fn pin(cpu: usize) {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

fn make_ring(payload: usize) -> (Arc<BroadcastTopicRing>, &'static mut [u64]) {
    let ring_bytes = BroadcastTopicRing::slot_stride(payload) * 64;
    let total = BroadcastTopicRing::required_memory_size(ring_bytes, payload);
    let mem: &'static mut [u64] = Box::leak(vec![0u64; total / 8 + 1].into_boxed_slice());
    let ptr = std::ptr::NonNull::new(mem.as_mut_ptr() as *mut u8).unwrap();
    let ring = unsafe {
        BroadcastTopicRing::from_memory(
            ptr,
            mem.len() * 8,
            ring_bytes,
            payload,
            Arc::new(TopicStats::default()),
        )
        .unwrap()
    };
    (Arc::new(ring), mem)
}

fn report(name: &str, mut cycles: Vec<u64>, tpn: f64) {
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

fn raw_floor(a: usize, b: usize, tpn: f64) {
    let ping = Arc::new(Line(AtomicU64::new(0)));
    let pong = Arc::new(Line(AtomicU64::new(0)));
    let total = WARMUP + SAMPLES;
    let (p2, q2) = (ping.clone(), pong.clone());
    let echo = thread::spawn(move || {
        pin(b);
        for i in 1..=total as u64 {
            while p2.0.load(Ordering::Acquire) != i {
                std::hint::spin_loop();
            }
            q2.0.store(i, Ordering::Release);
        }
    });
    pin(a);
    let mut out = Vec::with_capacity(SAMPLES);
    for i in 1..=total as u64 {
        let t0 = ticks();
        ping.0.store(i, Ordering::Release);
        while pong.0.load(Ordering::Acquire) != i {
            std::hint::spin_loop();
        }
        let t1 = ticks();
        if i as usize > WARMUP {
            out.push(t1 - t0);
        }
    }
    echo.join().unwrap();
    report("raw cache-line floor (2 atomics)", out, tpn);
}

fn ring_pingpong(a: usize, b: usize, payload: usize, tpn: f64) {
    let (ab, _m1) = make_ring(payload);
    let (ba, _m2) = make_ring(payload);
    let total = WARMUP + SAMPLES;
    let mut hdr = MessageHeader::new(1, 0, 8);
    hdr.set_format_type(FORMAT_TYPE_INLINE);
    hdr.set_no_checksum();
    let (ab2, ba2) = (ab.clone(), ba.clone());
    let mut cur_ab = ab.cursor_latest();
    let mut cur_ba = ba.cursor_latest();
    let echo = thread::spawn(move || {
        pin(b);
        let mut buf = [0u8; 64];
        for _ in 0..total {
            loop {
                if let Ok(ReadInto::Message { .. }) = ab2.read_into(&mut cur_ab, &mut buf) {
                    break;
                }
                std::hint::spin_loop();
            }
            let r = ba2.reserve(8).unwrap();
            unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), r.payload_ptr(), 8) };
            ba2.commit(r, &hdr, 8).unwrap();
        }
    });
    pin(a);
    let mut buf = [0u8; 64];
    let mut out = Vec::with_capacity(SAMPLES);
    for i in 0..total {
        let t0 = ticks();
        let r = ab.reserve(8).unwrap();
        unsafe { std::ptr::write_unaligned(r.payload_ptr() as *mut u64, i as u64) };
        ab.commit(r, &hdr, 8).unwrap();
        loop {
            if let Ok(ReadInto::Message { .. }) = ba.read_into(&mut cur_ba, &mut buf) {
                break;
            }
            std::hint::spin_loop();
        }
        let t1 = ticks();
        if i >= WARMUP {
            out.push(t1 - t0);
        }
    }
    echo.join().unwrap();
    report(&format!("broadcast ring ({payload} B payload)"), out, tpn);
}

#[repr(C, align(64))]
struct LeanSlot {
    seq: AtomicU64,
    data: [AtomicU64; 7],
}

struct LeanRing {
    slots: Box<[LeanSlot]>,
    mask: u64,
}

impl LeanRing {
    fn new(n: usize) -> Arc<Self> {
        let slots: Vec<LeanSlot> = (0..n)
            .map(|_| LeanSlot {
                seq: AtomicU64::new(0),
                data: Default::default(),
            })
            .collect();
        Arc::new(Self {
            slots: slots.into_boxed_slice(),
            mask: n as u64 - 1,
        })
    }

    #[inline(always)]
    fn publish(&self, idx: u64, value: u64) {
        let slot = &self.slots[(idx & self.mask) as usize];
        slot.seq.store(2 * idx - 1, Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::Release);
        slot.data[0].store(value, Ordering::Relaxed);
        slot.seq.store(2 * idx, Ordering::Release);
    }

    #[inline(always)]
    fn poll(&self, idx: u64) -> Option<u64> {
        let slot = &self.slots[(idx & self.mask) as usize];
        let s1 = slot.seq.load(Ordering::Acquire);
        if s1 != 2 * idx {
            return None;
        }
        let v = slot.data[0].load(Ordering::Relaxed);
        std::sync::atomic::fence(Ordering::Acquire);
        if slot.seq.load(Ordering::Relaxed) == s1 {
            Some(v)
        } else {
            None
        }
    }
}

fn lean_pingpong(a: usize, b: usize, tpn: f64) {
    let ab = LeanRing::new(64);
    let ba = LeanRing::new(64);
    let total = WARMUP + SAMPLES;
    let (ab2, ba2) = (ab.clone(), ba.clone());
    let echo = thread::spawn(move || {
        pin(b);
        for i in 1..=total as u64 {
            let v = loop {
                if let Some(v) = ab2.poll(i) {
                    break v;
                }
                std::hint::spin_loop();
            };
            ba2.publish(i, v);
        }
    });
    pin(a);
    let mut out = Vec::with_capacity(SAMPLES);
    for i in 1..=total as u64 {
        let t0 = ticks();
        ab.publish(i, i);
        loop {
            if ba.poll(i).is_some() {
                break;
            }
            std::hint::spin_loop();
        }
        let t1 = ticks();
        if i as usize > WARMUP {
            out.push(t1 - t0);
        }
    }
    echo.join().unwrap();
    report("lean 64 B slot prototype", out, tpn);
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let (a, b) = (
        args.first().copied().unwrap_or(2),
        args.get(1).copied().unwrap_or(4),
    );
    let tpn = ticks_per_ns();
    println!("cores {a} <-> {b}, {tpn:.3} ticks/ns, {WARMUP} warmup, {SAMPLES} samples");
    raw_floor(a, b, tpn);
    lean_pingpong(a, b, tpn);
    ring_pingpong(a, b, 56, tpn);
    ring_pingpong(a, b, 8, tpn);
}
