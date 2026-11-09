mod support;

use std::sync::{Arc, Barrier};
use std::thread;

use iceoryx2::prelude::*;
use renoir::topic_rings::FrameChannel;
use support::timing::*;

const SLOTS: usize = 4;

fn touch_write(buf: &mut [u8]) {
    let n = buf.len().min(64);
    buf[..n].fill(0xA5);
}

fn touch_read(buf: &[u8]) -> u8 {
    let n = buf.len().min(64);
    buf[..n].iter().fold(0u8, |a, b| a ^ b)
}

fn renoir_frames(a: usize, b: usize, payload: usize, touch: bool, tpn: f64) {
    let (warmup, samples) = samples_for(payload);
    let total = warmup + samples;
    let tag = format!("{}_{payload}_{touch}", std::process::id());
    let (ab, ba) = (format!("fr_ab_{tag}"), format!("fr_ba_{tag}"));
    let mut pub_ab = FrameChannel::open(&ab, payload, SLOTS, false, false, false).unwrap();
    let mut sub_ba = FrameChannel::open(&ba, payload, SLOTS, false, true, false).unwrap();
    let ready = Arc::new(Barrier::new(2));
    let echo_ready = ready.clone();
    let (ab2, ba2) = (ab.clone(), ba.clone());
    let echo = thread::spawn(move || {
        pin(b);
        let mut sub_ab = FrameChannel::open(&ab2, payload, SLOTS, false, true, false).unwrap();
        let mut pub_ba = FrameChannel::open(&ba2, payload, SLOTS, false, false, false).unwrap();
        echo_ready.wait();
        for _ in 0..total {
            loop {
                if let Some(view) = sub_ab.view().unwrap() {
                    if touch {
                        std::hint::black_box(touch_read(view.bytes()));
                    }
                    break;
                }
                std::hint::spin_loop();
            }
            let mut loan = loop {
                if let Ok(loan) = pub_ba.loan(payload) {
                    break loan;
                }
                std::hint::spin_loop();
            };
            if touch {
                touch_write(loan.bytes_mut());
            }
            loan.commit().unwrap();
        }
    });
    ready.wait();
    pin(a);
    let mut out = Vec::with_capacity(samples);
    for i in 0..total {
        let t0 = ticks();
        let mut loan = loop {
            if let Ok(loan) = pub_ab.loan(payload) {
                break loan;
            }
            std::hint::spin_loop();
        };
        if touch {
            touch_write(loan.bytes_mut());
        }
        loan.commit().unwrap();
        loop {
            if let Some(view) = sub_ba.view().unwrap() {
                if touch {
                    std::hint::black_box(touch_read(view.bytes()));
                }
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
    let mode = if touch { "touch 64 B" } else { "hand-over" };
    report(&format!("renoir frame {payload} B ({mode})"), out, tpn);
}

fn service_name(tag: &str, payload: usize, touch: bool) -> ServiceName {
    format!("renoir_fr/{}/{tag}/{payload}/{touch}", std::process::id())
        .as_str()
        .try_into()
        .unwrap()
}

fn iox_zero_copy(a: usize, b: usize, payload: usize, touch: bool, tpn: f64) {
    let (warmup, samples) = samples_for(payload);
    let total = warmup + samples;
    let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
    let ab = node
        .service_builder(&service_name("ab", payload, touch))
        .publish_subscribe::<[u8]>()
        .open_or_create()
        .unwrap();
    let ba = node
        .service_builder(&service_name("ba", payload, touch))
        .publish_subscribe::<[u8]>()
        .open_or_create()
        .unwrap();
    let publisher = ab.publisher_builder().initial_max_slice_len(payload).create().unwrap();
    let subscriber = ba.subscriber_builder().create().unwrap();
    let (ab_name, ba_name) = (service_name("ab", payload, touch), service_name("ba", payload, touch));
    let ready = Arc::new(Barrier::new(2));
    let echo_ready = ready.clone();
    let echo = thread::spawn(move || {
        pin(b);
        let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
        let ab = node
            .service_builder(&ab_name)
            .publish_subscribe::<[u8]>()
            .open_or_create()
            .unwrap();
        let ba = node
            .service_builder(&ba_name)
            .publish_subscribe::<[u8]>()
            .open_or_create()
            .unwrap();
        let subscriber = ab.subscriber_builder().create().unwrap();
        let publisher = ba.publisher_builder().initial_max_slice_len(payload).create().unwrap();
        echo_ready.wait();
        for _ in 0..total {
            let sample = loop {
                if let Some(s) = subscriber.receive().unwrap() {
                    break s;
                }
                std::hint::spin_loop();
            };
            if touch {
                std::hint::black_box(touch_read(sample.payload()));
            }
            drop(sample);
            let out = publisher.loan_slice_uninit(payload).unwrap();
            let mut out = unsafe { out.assume_init() };
            if touch {
                touch_write(out.payload_mut());
            }
            out.send().unwrap();
        }
    });
    ready.wait();
    pin(a);
    let mut out = Vec::with_capacity(samples);
    for i in 0..total {
        let t0 = ticks();
        let sample = publisher.loan_slice_uninit(payload).unwrap();
        let mut sample = unsafe { sample.assume_init() };
        if touch {
            touch_write(sample.payload_mut());
        }
        sample.send().unwrap();
        loop {
            if let Some(s) = subscriber.receive().unwrap() {
                if touch {
                    std::hint::black_box(touch_read(s.payload()));
                }
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
    let mode = if touch { "touch 64 B" } else { "hand-over" };
    report(&format!("iceoryx2 zero-copy {payload} B ({mode})"), out, tpn);
}

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (a, b) = (args.first().copied().unwrap_or(2), args.get(1).copied().unwrap_or(4));
    let tpn = ticks_per_ns();
    println!("cores {a} <-> {b}, {tpn:.3} ticks/ns");
    for payload in [56usize, 1024, 65_536, 921_600] {
        for touch in [false, true] {
            renoir_frames(a, b, payload, touch, tpn);
            iox_zero_copy(a, b, payload, touch, tpn);
        }
    }
}
