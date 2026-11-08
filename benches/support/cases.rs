use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::thread;

use renoir::ffi::{renoir_lean_close, renoir_lean_open, renoir_lean_publish, renoir_lean_try_read};
use renoir::topic::{MessageHeader, TopicStats, FORMAT_TYPE_INLINE};
use renoir::topic_rings::{
    lean_slot_bytes, BroadcastTopicRing, LeanRead, LeanRing, ReadInto, ViewRead,
};

use super::timing::*;

#[repr(align(64))]
struct Line(AtomicU64);

pub fn make_ring(payload: usize) -> (Arc<BroadcastTopicRing>, &'static mut [u64]) {
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

pub fn ring_view_pingpong(a: usize, b: usize, payload: usize, copy_payload: bool, tpn: f64) {
    let (ab, _m1) = make_ring(payload);
    let (ba, _m2) = make_ring(payload);
    let (warmup, samples) = samples_for(payload);
    let total = warmup + samples;
    let mut hdr = MessageHeader::new(1, 0, payload as u32);
    hdr.set_format_type(FORMAT_TYPE_INLINE);
    hdr.set_no_checksum();
    let (ab2, ba2) = (ab.clone(), ba.clone());
    let mut cur_ab = ab.cursor_latest();
    let mut cur_ba = ba.cursor_latest();
    let echo = thread::spawn(move || {
        pin(b);
        for _ in 0..total {
            let view = loop {
                if let Ok(ViewRead::View(v)) = ab2.view(&mut cur_ab) {
                    break v;
                }
                std::hint::spin_loop();
            };
            let r = ba2.reserve(payload).unwrap();
            if copy_payload {
                unsafe { std::ptr::copy_nonoverlapping(view.data_ptr(), r.payload_ptr(), payload) };
            }
            ab2.release_view(&mut cur_ab, &view).unwrap();
            ba2.commit(r, &hdr, payload).unwrap();
        }
    });
    pin(a);
    let source = aligned_buf(payload, 7);
    let mut out = Vec::with_capacity(samples);
    for i in 0..total {
        let t0 = ticks();
        let r = ab.reserve(payload).unwrap();
        if copy_payload {
            unsafe { std::ptr::copy_nonoverlapping(source.as_ptr(), r.payload_ptr(), payload) };
        }
        ab.commit(r, &hdr, payload).unwrap();
        let view = loop {
            if let Ok(ViewRead::View(v)) = ba.view(&mut cur_ba) {
                break v;
            }
            std::hint::spin_loop();
        };
        ba.release_view(&mut cur_ba, &view).unwrap();
        let t1 = ticks();
        if i >= warmup {
            out.push(t1 - t0);
        }
    }
    echo.join().unwrap();
    let mode = if copy_payload {
        "copy in, view out"
    } else {
        "view only"
    };
    report(&format!("broadcast view {payload} B ({mode})"), out, tpn);
}

pub fn raw_floor(a: usize, b: usize, tpn: f64) {
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

pub fn ring_pingpong(a: usize, b: usize, payload: usize, tpn: f64) {
    let (ab, _m1) = make_ring(payload);
    let (ba, _m2) = make_ring(payload);
    let (warmup, samples) = samples_for(payload);
    let total = warmup + samples;
    let mut hdr = MessageHeader::new(1, 0, payload as u32);
    hdr.set_format_type(FORMAT_TYPE_INLINE);
    hdr.set_no_checksum();
    let (ab2, ba2) = (ab.clone(), ba.clone());
    let mut cur_ab = ab.cursor_latest();
    let mut cur_ba = ba.cursor_latest();
    let echo = thread::spawn(move || {
        pin(b);
        let mut buf = aligned_buf(payload, 0);
        for _ in 0..total {
            loop {
                if let Ok(ReadInto::Message { .. }) = ab2.read_into(&mut cur_ab, &mut buf) {
                    break;
                }
                std::hint::spin_loop();
            }
            let r = ba2.reserve(payload).unwrap();
            unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), r.payload_ptr(), payload) };
            ba2.commit(r, &hdr, payload).unwrap();
        }
    });
    pin(a);
    let mut buf = aligned_buf(payload, 0);
    let source = aligned_buf(payload, 7);
    let mut out = Vec::with_capacity(samples);
    for i in 0..total {
        let t0 = ticks();
        let r = ab.reserve(payload).unwrap();
        unsafe { std::ptr::copy_nonoverlapping(source.as_ptr(), r.payload_ptr(), payload) };
        ab.commit(r, &hdr, payload).unwrap();
        loop {
            if let Ok(ReadInto::Message { .. }) = ba.read_into(&mut cur_ba, &mut buf) {
                break;
            }
            std::hint::spin_loop();
        }
        let t1 = ticks();
        if i >= warmup {
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

pub fn lean_pingpong(a: usize, b: usize, tpn: f64) {
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

pub fn real_lean_pingpong(a: usize, b: usize, msg: usize, tpn: f64) {
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

pub fn ffi_pingpong(a: usize, b: usize, msg: usize, tpn: f64) {
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
