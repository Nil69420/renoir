use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::thread;

use renoir::ffi::{renoir_lean_close, renoir_lean_open, renoir_lean_publish, renoir_lean_try_read};
use renoir::topic::{MessageHeader, TopicStats, FORMAT_TYPE_INLINE};
use renoir::topic_rings::{lean_slot_bytes, BroadcastTopicRing, LeanRead, LeanRing, ReadInto};

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

#[repr(align(128))]
struct Pad([u8; 512]);

struct AlignedBuf {
    pad: Box<Pad>,
    len: usize,
}

impl std::ops::Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.pad.0[..self.len]
    }
}

impl std::ops::DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.pad.0[..self.len]
    }
}

fn aligned_buf(len: usize, fill: u8) -> AlignedBuf {
    AlignedBuf {
        pad: Box::new(Pad([fill; 512])),
        len,
    }
}

fn aligned_mem(total: usize) -> (std::ptr::NonNull<u8>, &'static mut [u64]) {
    let mem: &'static mut [u64] = Box::leak(vec![0u64; total / 8 + 1537].into_boxed_slice());
    let base = mem.as_mut_ptr() as usize;
    let skew: usize = std::env::var("BENCH_SKEW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let aligned = ((base + 4095) & !4095) + if call % 2 == 1 { skew } else { 0 };
    (std::ptr::NonNull::new(aligned as *mut u8).unwrap(), mem)
}

fn make_ring(payload: usize) -> (Arc<BroadcastTopicRing>, &'static mut [u64]) {
    let ring_bytes = BroadcastTopicRing::slot_stride(payload) * 64;
    let total = BroadcastTopicRing::required_memory_size(ring_bytes, payload);
    let (ptr, mem) = aligned_mem(total);
    let ring = unsafe {
        BroadcastTopicRing::from_memory(
            ptr,
            total,
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
struct ProtoSlot {
    seq: AtomicU64,
    data: [AtomicU64; 7],
}

struct ProtoRing {
    slots: Box<[ProtoSlot]>,
    mask: u64,
}

impl ProtoRing {
    fn new(n: usize) -> Arc<Self> {
        let slots: Vec<ProtoSlot> = (0..n)
            .map(|_| ProtoSlot {
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
    let ab = ProtoRing::new(64);
    let ba = ProtoRing::new(64);
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

fn make_lean(msg: usize) -> (Arc<LeanRing>, &'static mut [u64]) {
    let ring_bytes = lean_slot_bytes(msg).unwrap() * 64;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let (ptr, mem) = aligned_mem(total);
    let ring = unsafe { LeanRing::from_memory(ptr, total, ring_bytes, msg).unwrap() };
    (Arc::new(ring), mem)
}

#[repr(align(128))]
struct Side {
    cursor: renoir::topic_rings::LeanCursor,
    publisher: renoir::topic_rings::LeanPublisher,
}

fn real_lean_pingpong(a: usize, b: usize, msg: usize, tpn: f64) {
    let (ab, _m1) = make_lean(msg);
    let (ba, _m2) = make_lean(msg);
    let total = WARMUP + SAMPLES;
    let (ab2, ba2) = (ab.clone(), ba.clone());
    let mut main_side = Box::new(Side {
        cursor: ba.cursor_latest(),
        publisher: ab.publisher(),
    });
    let mut echo_side = Box::new(Side {
        cursor: ab.cursor_latest(),
        publisher: ba2.publisher(),
    });
    let echo = thread::spawn(move || {
        pin(b);
        let mut buf = aligned_buf(msg, 0);
        for _ in 0..total {
            loop {
                if let Ok(LeanRead::Message) = ab2.try_read(&mut echo_side.cursor, &mut buf) {
                    break;
                }
                std::hint::spin_loop();
            }
            echo_side.publisher.publish(&buf).unwrap();
        }
    });
    pin(a);
    let mut buf = aligned_buf(msg, 0);
    let payload = aligned_buf(msg, 7);
    let mut out = Vec::with_capacity(SAMPLES);
    for i in 0..total {
        let t0 = ticks();
        main_side.publisher.publish(&payload).unwrap();
        loop {
            if let Ok(LeanRead::Message) = ba.try_read(&mut main_side.cursor, &mut buf) {
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
    report(&format!("LeanRing product ({msg} B message)"), out, tpn);
}

fn ffi_pingpong(a: usize, b: usize, msg: usize, tpn: f64) {
    let dir = std::env::temp_dir().join(format!("renoir_bench_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("RENOIR_SHM_DIR", &dir);
    let open = |n: &str, from_oldest: u32| {
        let name = std::ffi::CString::new(n).unwrap();
        let mut h = std::ptr::null_mut();
        let rc = renoir_lean_open(name.as_ptr(), msg as u32, 64, from_oldest, &mut h);
        assert_eq!(rc as i32, 0);
        h as usize
    };
    let (ab_p, ab_s, ba_p, ba_s) = (open("ab", 0), open("ab", 0), open("ba", 0), open("ba", 0));
    let total = WARMUP + SAMPLES;
    let echo = thread::spawn(move || {
        pin(b);
        let mut buf = aligned_buf(msg, 0);
        for _ in 0..total {
            while renoir_lean_try_read(ab_s as _, buf.as_mut_ptr(), msg) as i32 != 0 {
                std::hint::spin_loop();
            }
            renoir_lean_publish(ba_p as _, buf.as_ptr(), msg);
        }
    });
    pin(a);
    let mut buf = aligned_buf(msg, 0);
    let payload = aligned_buf(msg, 7);
    let mut out = Vec::with_capacity(SAMPLES);
    for i in 0..total {
        let t0 = ticks();
        renoir_lean_publish(ab_p as _, payload.as_ptr(), msg);
        while renoir_lean_try_read(ba_s as _, buf.as_mut_ptr(), msg) as i32 != 0 {
            std::hint::spin_loop();
        }
        let t1 = ticks();
        if i >= WARMUP {
            out.push(t1 - t0);
        }
    }
    echo.join().unwrap();
    for h in [ab_p, ab_s, ba_p, ba_s] {
        renoir_lean_close(h as _);
    }
    let _ = std::fs::remove_dir_all(&dir);
    report(&format!("C ABI lean ({msg} B message)"), out, tpn);
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
    real_lean_pingpong(a, b, 56, tpn);
    real_lean_pingpong(a, b, 48, tpn);
    real_lean_pingpong(a, b, 16, tpn);
    real_lean_pingpong(a, b, 120, tpn);
    ffi_pingpong(a, b, 56, tpn);
    ffi_pingpong(a, b, 16, tpn);
    ring_pingpong(a, b, 56, tpn);
    ring_pingpong(a, b, 8, tpn);
}
