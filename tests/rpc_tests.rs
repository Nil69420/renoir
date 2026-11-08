use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use renoir::rpc::{RpcClient, RpcServer, RPC_MAX_PAYLOAD};

fn shm_dir() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    });
}

fn unique(tag: &str) -> String {
    use std::sync::atomic::AtomicU32;
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "rpc_{tag}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn serve(service: String, max: usize, stop: Arc<AtomicBool>) -> thread::JoinHandle<u64> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let mut server = RpcServer::open(&service, max, 64).unwrap();
        ready_tx.send(()).unwrap();
        let mut buf = vec![0u8; max];
        let mut served = 0u64;
        while !stop.load(Ordering::Relaxed) {
            match server.poll(&mut buf).unwrap() {
                Some(req) => {
                    let reply: Vec<u8> = buf[..req.len].iter().map(|b| b ^ 0xFF).collect();
                    server.reply(req.client_id, req.sequence, &reply).unwrap();
                    served += 1;
                }
                None => {
                    server.wait_request(100, Some(Duration::from_millis(5)));
                }
            }
        }
        served
    });
    ready_rx.recv().unwrap();
    handle
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn single_client_gets_its_own_transformed_responses() {
    shm_dir();
    let service = unique("single");
    let stop = Arc::new(AtomicBool::new(false));
    let server = serve(service.clone(), 64, stop.clone());
    let mut client = RpcClient::open(&service, 64, 64).unwrap();
    let mut out = [0u8; 64];
    for i in 0..500u32 {
        let request = i.to_le_bytes();
        let n = client
            .call(&request, &mut out, Duration::from_secs(5))
            .unwrap();
        assert_eq!(n, 4);
        let expected: Vec<u8> = request.iter().map(|b| b ^ 0xFF).collect();
        assert_eq!(&out[..n], &expected[..]);
    }
    stop.store(true, Ordering::Relaxed);
    assert_eq!(server.join().unwrap(), 500);
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn many_clients_never_see_each_others_responses() {
    shm_dir();
    let service = unique("many");
    let stop = Arc::new(AtomicBool::new(false));
    let server = serve(service.clone(), 64, stop.clone());
    let calls = if cfg!(miri) { 5 } else { 400 };
    let clients: Vec<_> = (0..4u8)
        .map(|c| {
            let service = service.clone();
            thread::spawn(move || {
                let mut client = RpcClient::open(&service, 64, 64).unwrap();
                let mut out = [0u8; 64];
                for i in 0..calls {
                    let request = [c, (i % 251) as u8, 9, 9, 9];
                    let n = client
                        .call(&request, &mut out, Duration::from_secs(10))
                        .unwrap();
                    let expected: Vec<u8> = request.iter().map(|b| b ^ 0xFF).collect();
                    assert_eq!(&out[..n], &expected[..], "client {c} call {i}");
                }
            })
        })
        .collect();
    for c in clients {
        c.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    assert_eq!(server.join().unwrap(), 4 * calls as u64);
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn call_times_out_without_a_server() {
    shm_dir();
    let mut client = RpcClient::open(&unique("noserver"), 32, 8).unwrap();
    let mut out = [0u8; 32];
    let start = Instant::now();
    assert!(client
        .call(b"hello", &mut out, Duration::from_millis(60))
        .is_err());
    assert!(start.elapsed() >= Duration::from_millis(55));
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn oversized_and_invalid_sizes_are_rejected() {
    shm_dir();
    assert!(RpcClient::open(&unique("zero"), 0, 8).is_err());
    assert!(RpcClient::open(&unique("big"), RPC_MAX_PAYLOAD + 1, 8).is_err());
    assert!(RpcClient::open(&unique("slots"), 32, 1).is_err());
    let mut client = RpcClient::open(&unique("limit"), 8, 8).unwrap();
    let mut out = [0u8; 8];
    assert!(client
        .call(&[0u8; 9], &mut out, Duration::from_millis(10))
        .is_err());
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn maximum_payload_round_trips() {
    shm_dir();
    let service = unique("max");
    let stop = Arc::new(AtomicBool::new(false));
    let server = serve(service.clone(), RPC_MAX_PAYLOAD, stop.clone());
    let mut client = RpcClient::open(&service, RPC_MAX_PAYLOAD, 64).unwrap();
    let request: Vec<u8> = (0..RPC_MAX_PAYLOAD).map(|i| i as u8).collect();
    let mut out = vec![0u8; RPC_MAX_PAYLOAD];
    let n = client
        .call(&request, &mut out, Duration::from_secs(5))
        .unwrap();
    assert_eq!(n, RPC_MAX_PAYLOAD);
    assert!(out.iter().zip(&request).all(|(o, r)| *o == r ^ 0xFF));
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
}
