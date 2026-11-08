mod support;

use std::sync::{Arc, Barrier};
use std::thread;

use iceoryx2::prelude::*;
use support::cases::*;
use support::timing::*;

fn service_name(tag: &str, payload: usize) -> ServiceName {
    format!("renoir_bench/{}/{tag}/{payload}", std::process::id())
        .as_str()
        .try_into()
        .unwrap()
}

fn iox_pingpong(a: usize, b: usize, payload: usize, copy_payload: bool, tpn: f64) {
    let (warmup, samples) = samples_for(payload);
    let total = warmup + samples;
    let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
    let ab = node
        .service_builder(&service_name("ab", payload))
        .publish_subscribe::<[u8]>()
        .open_or_create()
        .unwrap();
    let ba = node
        .service_builder(&service_name("ba", payload))
        .publish_subscribe::<[u8]>()
        .open_or_create()
        .unwrap();
    let publisher = ab
        .publisher_builder()
        .initial_max_slice_len(payload)
        .create()
        .unwrap();
    let subscriber = ba.subscriber_builder().create().unwrap();

    let ab_name = service_name("ab", payload);
    let ba_name = service_name("ba", payload);
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
        let publisher = ba
            .publisher_builder()
            .initial_max_slice_len(payload)
            .create()
            .unwrap();
        echo_ready.wait();
        for _ in 0..total {
            let sample = loop {
                if let Some(s) = subscriber.receive().unwrap() {
                    break s;
                }
                std::hint::spin_loop();
            };
            let out = publisher.loan_slice_uninit(payload).unwrap();
            if copy_payload {
                out.write_from_slice(sample.payload()).send().unwrap();
            } else {
                unsafe { out.assume_init() }.send().unwrap();
            }
        }
    });

    ready.wait();
    pin(a);
    let source = aligned_buf(payload, 7);
    let mut out = Vec::with_capacity(samples);
    for i in 0..total {
        let t0 = ticks();
        let sample = publisher.loan_slice_uninit(payload).unwrap();
        if copy_payload {
            sample.write_from_slice(&source).send().unwrap();
        } else {
            unsafe { sample.assume_init() }.send().unwrap();
        }
        loop {
            if let Some(s) = subscriber.receive().unwrap() {
                drop(s);
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
    let mode = if copy_payload {
        "copy in/out"
    } else {
        "zero-copy"
    };
    report(&format!("iceoryx2 {payload} B ({mode})"), out, tpn);
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
    println!("cores {a} <-> {b}, {tpn:.3} ticks/ns");
    raw_floor(a, b, tpn);
    real_lean_pingpong(a, b, 56, tpn);
    ffi_pingpong(a, b, 56, tpn);
    for payload in [56usize, 1024, 65_536, 921_600] {
        ring_pingpong(a, b, payload, tpn);
        iox_pingpong(a, b, payload, true, tpn);
        iox_pingpong(a, b, payload, false, tpn);
    }
}
