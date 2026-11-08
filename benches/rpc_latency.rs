mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use renoir::rpc::{RpcClient, RpcServer};
use support::timing::*;

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
    let dir = std::env::temp_dir().join(format!("renoir_rpc_bench_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("RENOIR_SHM_DIR", &dir);
    println!("cores {a} <-> {b}, {tpn:.3} ticks/ns");
    for payload in [8usize, 64, 256] {
        let service = format!("rpc_bench_{payload}");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_server = stop.clone();
        let name = service.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            pin(b);
            let mut server = RpcServer::open(&name, payload, 64).unwrap();
            ready_tx.send(()).unwrap();
            let mut buf = vec![0u8; payload];
            while !stop_server.load(Ordering::Relaxed) {
                if let Some(req) = server.poll(&mut buf).unwrap() {
                    server
                        .reply(req.client_id, req.sequence, &buf[..req.len])
                        .unwrap();
                } else {
                    std::hint::spin_loop();
                }
            }
        });
        ready_rx.recv().unwrap();
        pin(a);
        let mut client = RpcClient::open(&service, payload, 64).unwrap();
        let request = vec![7u8; payload];
        let mut response = vec![0u8; payload];
        let (warmup, samples) = samples_for(payload);
        let mut out = Vec::with_capacity(samples);
        for i in 0..warmup + samples {
            let t0 = ticks();
            client
                .call(&request, &mut response, Duration::from_secs(5))
                .unwrap();
            let t1 = ticks();
            if i >= warmup {
                out.push(t1 - t0);
            }
        }
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
        report(&format!("rpc call ({payload} B payload)"), out, tpn);
    }
}
