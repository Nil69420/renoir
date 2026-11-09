use std::time::{Duration, Instant};

use renoir::rpc::{RpcClient, RpcServer};

fn unique(tag: &str) -> String {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    });
    format!("rpcq_{tag}_{}", std::process::id())
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn a_registered_server_that_falls_behind_drops_stale_stamped_requests_only() {
    let service = unique("stale");
    let mut server = RpcServer::open(&service, 32, 16).unwrap();
    server.set_request_lifespan(Some(ms(50)));
    let mut client = RpcClient::open(&service, 32, 16).unwrap();
    let mut response = [0u8; 32];

    client.set_stamp_requests(true);
    assert!(client.call(b"old-stamped", &mut response, ms(20)).is_err());
    client.set_stamp_requests(false);
    assert!(client
        .call(b"old-unstamped", &mut response, ms(20))
        .is_err());
    std::thread::sleep(ms(120));

    let mut buf = [0u8; 32];
    let first = server
        .poll(&mut buf)
        .unwrap()
        .expect("the unstamped request");
    assert_eq!(&buf[..first.len], b"old-unstamped");
    assert_eq!(server.stale_requests(), 1);
    assert!(server.poll(&mut buf).unwrap().is_none());

    client.set_stamp_requests(true);
    let waiter = std::thread::spawn(move || {
        let mut out = [0u8; 32];
        let n = client
            .call(b"fresh", &mut out, Duration::from_secs(5))
            .unwrap();
        out[..n].to_vec()
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(req) = server.poll(&mut buf).unwrap() {
            assert_eq!(&buf[..req.len], b"fresh");
            server.reply(req.client_id, req.sequence, b"ok").unwrap();
            break;
        }
        assert!(Instant::now() < deadline, "the fresh request never arrived");
        std::hint::spin_loop();
    }
    assert_eq!(waiter.join().unwrap(), b"ok");
    assert_eq!(server.stale_requests(), 1);
}

#[test]
fn a_client_can_ask_whether_a_server_is_alive_and_wait_for_one() {
    let service = unique("alive");
    let mut client = RpcClient::open(&service, 32, 16).unwrap();
    assert_eq!(client.servers_alive().unwrap(), 0);
    let started = Instant::now();
    assert!(!client.wait_for_server(Some(ms(30))).unwrap());
    assert!(started.elapsed() >= ms(25));

    let name = service.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(ms(40));
        RpcServer::open(&name, 32, 16).unwrap()
    });
    assert!(client
        .wait_for_server(Some(Duration::from_secs(5)))
        .unwrap());
    let server = handle.join().unwrap();
    assert_eq!(client.servers_alive().unwrap(), 1);
    drop(server);
    assert_eq!(client.servers_alive().unwrap(), 0);
}

#[test]
fn a_request_sent_before_any_server_exists_is_not_delivered_to_a_server_that_starts_later() {
    let service = unique("early");
    let mut client = RpcClient::open(&service, 32, 16).unwrap();
    let mut response = [0u8; 32];
    assert!(client.call(b"too-early", &mut response, ms(20)).is_err());
    let mut server = RpcServer::open(&service, 32, 16).unwrap();
    let mut buf = [0u8; 32];
    assert!(server.poll(&mut buf).unwrap().is_none());
}
