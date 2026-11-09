use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use renoir::rpc::{RpcClient, RpcServer, RPC_MAX_SERVERS};

const ROLE: &str = "RENOIR_RPC_SHARD_ROLE";
const SERVICE: &str = "RENOIR_RPC_SHARD_SERVICE";

fn shm_dir() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    });
}

fn serve(mut server: RpcServer, stop: Arc<AtomicBool>, handled: Arc<AtomicU64>) {
    let mut buf = [0u8; 64];
    while !stop.load(Ordering::Relaxed) {
        match server.poll(&mut buf).unwrap() {
            Some(req) => {
                server
                    .reply(req.client_id, req.sequence, &buf[..req.len])
                    .unwrap();
                handled.fetch_add(1, Ordering::Relaxed);
            }
            None => {
                server.wait_request(200, Some(Duration::from_millis(2)));
            }
        }
    }
}

fn child_server() {
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
    let service = std::env::var(SERVICE).unwrap();
    let server = RpcServer::open(&service, 64, 32).unwrap();
    serve(
        server,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU64::new(0)),
    );
}

#[test]
fn each_server_owns_one_shard_and_a_freed_shard_can_be_retaken() {
    shm_dir();
    let service = format!("shards_own_{}", std::process::id());
    let servers: Vec<_> = (0..RPC_MAX_SERVERS)
        .map(|_| RpcServer::open(&service, 64, 32).unwrap())
        .collect();
    let mut shards: Vec<_> = servers.iter().map(RpcServer::shard).collect();
    shards.sort_unstable();
    assert_eq!(shards, (0..RPC_MAX_SERVERS).collect::<Vec<_>>());
    assert!(RpcServer::open(&service, 64, 32).is_err());
    let mut servers = servers;
    let freed = servers.swap_remove(1);
    let freed_shard = freed.shard();
    drop(freed);
    let again = RpcServer::open(&service, 64, 32).unwrap();
    assert_eq!(again.shard(), freed_shard);
}

#[test]
fn two_servers_share_the_clients_and_every_request_is_answered_exactly_once() {
    shm_dir();
    let service = format!("shards_share_{}", std::process::id());
    let stop = Arc::new(AtomicBool::new(false));
    let handled = [Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0))];
    let mut workers = Vec::new();
    for h in &handled {
        let server = RpcServer::open(&service, 64, 32).unwrap();
        let (stop, h) = (stop.clone(), h.clone());
        workers.push(thread::spawn(move || serve(server, stop, h)));
    }
    let mut clients = Vec::new();
    let mut seen = [false; 2];
    while clients.len() < 8 || !(seen[0] && seen[1]) {
        let client = RpcClient::open(&service, 64, 32).unwrap();
        seen[client.shard()] = true;
        clients.push(client);
        assert!(clients.len() < 256, "clients never spread over both shards");
    }
    let calls = 300u64;
    let total = clients.len() as u64 * calls;
    thread::scope(|scope| {
        for (n, mut client) in clients.into_iter().enumerate() {
            scope.spawn(move || {
                let mut out = [0u8; 64];
                for i in 0..calls {
                    let request = [(n as u8) ^ (i as u8); 24];
                    let len = client
                        .call(&request, &mut out, Duration::from_secs(10))
                        .unwrap();
                    assert_eq!(&out[..len], &request, "client {n} call {i}");
                }
            });
        }
    });
    stop.store(true, Ordering::Relaxed);
    for w in workers {
        w.join().unwrap();
    }
    let counts: Vec<u64> = handled.iter().map(|h| h.load(Ordering::Relaxed)).collect();
    assert_eq!(
        counts.iter().sum::<u64>(),
        total,
        "each request handled exactly once"
    );
    assert!(
        counts.iter().all(|c| *c > 0),
        "load must reach both servers: {counts:?}"
    );
}

#[test]
fn clients_fail_over_when_the_server_they_use_dies() {
    if std::env::var(ROLE).as_deref() == Ok("server") {
        child_server();
        return;
    }
    shm_dir();
    let service = format!("shards_failover_{}", std::process::id());
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "clients_fail_over_when_the_server_they_use_dies",
            "--nocapture",
        ])
        .env(ROLE, "server")
        .env(SERVICE, &service)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let ready = Instant::now() + Duration::from_secs(20);
    let mut client = loop {
        let candidate = RpcClient::open(&service, 64, 32).unwrap();
        let mut out = [0u8; 64];
        let mut probe = candidate;
        if probe
            .call(b"ping", &mut out, Duration::from_millis(300))
            .is_ok()
        {
            break probe;
        }
        assert!(Instant::now() < ready, "child server never answered");
    };
    assert_eq!(client.shard(), 0, "the child owns shard 0");

    let stop = Arc::new(AtomicBool::new(false));
    let handled = Arc::new(AtomicU64::new(0));
    let backup = RpcServer::open(&service, 64, 32).unwrap();
    assert_eq!(backup.shard(), 1);
    let worker = {
        let (stop, handled) = (stop.clone(), handled.clone());
        thread::spawn(move || serve(backup, stop, handled))
    };
    child.kill().unwrap();
    child.wait().unwrap();

    let mut out = [0u8; 64];
    for i in 0..50u8 {
        let request = [i; 16];
        let len = client
            .call(&request, &mut out, Duration::from_secs(10))
            .expect("call must fail over to the surviving server");
        assert_eq!(&out[..len], &request);
    }
    assert_eq!(client.shard(), 1);
    assert!(handled.load(Ordering::Relaxed) >= 50);
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
}
