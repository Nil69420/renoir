mod support;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use iceoryx2::prelude::*;
use renoir::topic_rings::{lean_slot_bytes, LeanRead, LeanRing};
use support::timing::*;

#[repr(align(128))]
struct Ack(AtomicU64);

fn acks(n: usize) -> Arc<Vec<Ack>> {
    Arc::new((0..n).map(|_| Ack(AtomicU64::new(0))).collect())
}

fn wait_all(acks: &[Ack], n: u64) {
    for a in acks {
        while a.0.load(Ordering::Acquire) < n {
            std::hint::spin_loop();
        }
    }
}

fn lean_fanout(publisher_cpu: usize, consumer_cpus: &[usize], msg: usize, tpn: f64) {
    let ring_bytes = lean_slot_bytes(msg).unwrap() * 64;
    let total_bytes = LeanRing::required_memory_size(ring_bytes, msg);
    let (ptr, _mem) = aligned_mem(total_bytes);
    let ring =
        Arc::new(unsafe { LeanRing::from_memory(ptr, total_bytes, ring_bytes, msg).unwrap() });
    let (warmup, samples) = samples_for(msg);
    let total = (warmup + samples) as u64;
    let ack = acks(consumer_cpus.len());
    let ready = Arc::new(Barrier::new(consumer_cpus.len() + 1));
    let mut handles = Vec::new();
    for (i, cpu) in consumer_cpus.iter().copied().enumerate() {
        let (ring, ack, ready) = (ring.clone(), ack.clone(), ready.clone());
        handles.push(thread::spawn(move || {
            pin(cpu);
            let mut cursor = ring.cursor_latest();
            let mut buf = aligned_buf(msg, 0);
            ready.wait();
            let mut seen = 0u64;
            while seen < total {
                if let Ok(LeanRead::Message) = ring.try_read(&mut cursor, &mut buf) {
                    seen += 1;
                    ack[i].0.store(seen, Ordering::Release);
                } else {
                    std::hint::spin_loop();
                }
            }
        }));
    }
    pin(publisher_cpu);
    let mut publisher = ring.publisher();
    let payload = aligned_buf(msg, 7);
    ready.wait();
    let mut out = Vec::with_capacity(samples);
    for n in 1..=total {
        let t0 = ticks();
        publisher.publish(&payload).unwrap();
        wait_all(&ack, n);
        let t1 = ticks();
        if n as usize > warmup {
            out.push(t1 - t0);
        }
    }
    for h in handles {
        h.join().unwrap();
    }
    report(
        &format!("renoir lean fan-out {} x {msg} B", consumer_cpus.len()),
        out,
        tpn,
    );
}

fn iox_fanout(publisher_cpu: usize, consumer_cpus: &[usize], msg: usize, tpn: f64) {
    let (warmup, samples) = samples_for(msg);
    let total = (warmup + samples) as u64;
    let name: ServiceName = format!(
        "renoir_fanout/{}/{}",
        std::process::id(),
        consumer_cpus.len()
    )
    .as_str()
    .try_into()
    .unwrap();
    let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
    let service = node
        .service_builder(&name)
        .publish_subscribe::<[u8]>()
        .max_subscribers(8)
        .open_or_create()
        .unwrap();
    let publisher = service
        .publisher_builder()
        .initial_max_slice_len(msg)
        .create()
        .unwrap();
    let ack = acks(consumer_cpus.len());
    let ready = Arc::new(Barrier::new(consumer_cpus.len() + 1));
    let mut handles = Vec::new();
    for (i, cpu) in consumer_cpus.iter().copied().enumerate() {
        let (ack, ready, name) = (ack.clone(), ready.clone(), name);
        handles.push(thread::spawn(move || {
            pin(cpu);
            let node = NodeBuilder::new().create::<ipc::Service>().unwrap();
            let service = node
                .service_builder(&name)
                .publish_subscribe::<[u8]>()
                .max_subscribers(8)
                .open_or_create()
                .unwrap();
            let subscriber = service.subscriber_builder().create().unwrap();
            ready.wait();
            let mut seen = 0u64;
            while seen < total {
                if let Some(sample) = subscriber.receive().unwrap() {
                    drop(sample);
                    seen += 1;
                    ack[i].0.store(seen, Ordering::Release);
                } else {
                    std::hint::spin_loop();
                }
            }
        }));
    }
    pin(publisher_cpu);
    let source = aligned_buf(msg, 7);
    ready.wait();
    let mut out = Vec::with_capacity(samples);
    for n in 1..=total {
        let t0 = ticks();
        let sample = publisher.loan_slice_uninit(msg).unwrap();
        sample.write_from_slice(&source).send().unwrap();
        wait_all(&ack, n);
        let t1 = ticks();
        if n as usize > warmup {
            out.push(t1 - t0);
        }
    }
    for h in handles {
        h.join().unwrap();
    }
    report(
        &format!("iceoryx2 fan-out {} x {msg} B", consumer_cpus.len()),
        out,
        tpn,
    );
}

fn main() {
    let cpus: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let cpus = if cpus.len() >= 2 {
        cpus
    } else {
        vec![2, 4, 6, 8]
    };
    let tpn = ticks_per_ns();
    println!(
        "publisher cpu {}, consumer cpus {:?}, {tpn:.3} ticks/ns",
        cpus[0],
        &cpus[1..]
    );
    for k in 1..cpus.len().min(4) {
        for msg in [56usize, 504] {
            lean_fanout(cpus[0], &cpus[1..=k], msg, tpn);
            iox_fanout(cpus[0], &cpus[1..=k], msg, tpn);
        }
    }
}
