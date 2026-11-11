use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use renoir::rpc::{RpcClient, RpcServer};
use renoir::topic_rings::LeanChannel;

const ROLE: &str = "RENOIR_RPC_LIFECYCLE_ROLE";
const SERVICE: &str = "RENOIR_RPC_LIFECYCLE_SERVICE";

fn shm_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<(Option<tempfile::TempDir>, PathBuf)> =
        std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        if std::env::var(ROLE).is_ok() {
            return (
                None,
                PathBuf::from(std::env::var("RENOIR_SHM_DIR").unwrap()),
            );
        }
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        let path = dir.path().to_path_buf();
        (Some(dir), path)
    })
    .1
    .clone()
}

fn unique(tag: &str) -> String {
    shm_dir();
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    format!(
        "life_{tag}_{}_{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn response_file(service: &str, client_id: u32) -> PathBuf {
    shm_dir().join(format!("renoir_{service}_rsp_{client_id:08x}"))
}

fn plant_orphan(service: &str, id: u32, age: Duration) -> PathBuf {
    let path = response_file(service, id);
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(4096).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
    path
}

fn count_responses(service: &str) -> usize {
    let prefix = format!("renoir_{service}_rsp_");
    std::fs::read_dir(shm_dir())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .count()
}

#[test]
fn a_client_that_closes_removes_its_response_ring() {
    let service = unique("close");
    let _server = RpcServer::open(&service, 32, 16).unwrap();
    let client = RpcClient::open(&service, 32, 16).unwrap();
    let path = response_file(&service, client.client_id());
    assert!(path.exists());
    drop(client);
    assert!(!path.exists(), "the response ring outlived its client");
}

#[test]
fn opening_a_client_or_server_removes_old_unheld_response_rings_only() {
    let service = unique("orphan");
    let old = plant_orphan(&service, 0xdead_0001, Duration::from_secs(60));
    let young = plant_orphan(&service, 0xdead_0002, Duration::ZERO);
    let other = unique("other");
    let foreign = plant_orphan(&other, 0xdead_0003, Duration::from_secs(60));

    let _server = RpcServer::open(&service, 32, 16).unwrap();
    assert!(!old.exists(), "an old unheld response ring was kept");
    assert!(young.exists(), "a young response ring was removed");
    assert!(foreign.exists(), "another service's ring was removed");

    let old_again = plant_orphan(&service, 0xdead_0004, Duration::from_secs(60));
    let _client = RpcClient::open(&service, 32, 16).unwrap();
    assert!(!old_again.exists(), "a client open did not sweep");
}

#[test]
fn a_live_client_keeps_its_response_ring_however_old_it_looks() {
    let service = unique("live");
    let _server = RpcServer::open(&service, 32, 16).unwrap();
    let client = RpcClient::open(&service, 32, 16).unwrap();
    let path = response_file(&service, client.client_id());
    let mut lease = path.as_os_str().to_owned();
    lease.push(".lease");
    for file in [path.clone(), PathBuf::from(lease)] {
        std::fs::OpenOptions::new()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
    }
    let _second = RpcClient::open(&service, 32, 16).unwrap();
    let _server_two = RpcServer::open(&service, 32, 16).unwrap();
    assert!(path.exists(), "a live client's ring was reaped");
    let mut out = [0u8; 32];
    let mut client = client;
    assert!(client
        .call(b"x", &mut out, Duration::from_millis(50))
        .is_err());
}

#[test]
fn a_killed_client_process_leaves_a_ring_that_the_next_open_reaps() {
    if let Ok(role) = std::env::var(ROLE) {
        assert_eq!(role, "client");
        let service = std::env::var(SERVICE).unwrap();
        let client = RpcClient::open(&service, 32, 16).unwrap();
        println!("{}", client.client_id());
        std::thread::sleep(Duration::from_secs(600));
        return;
    }
    let service = unique("kill");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_killed_client_process_leaves_a_ring_that_the_next_open_reaps",
            "--nocapture",
        ])
        .env(ROLE, "client")
        .env(SERVICE, &service)
        .env("RENOIR_SHM_DIR", shm_dir())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while count_responses(&service) == 0 {
        assert!(std::time::Instant::now() < deadline, "child never opened");
        std::thread::sleep(Duration::from_millis(20));
    }
    let path = std::fs::read_dir(shm_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.starts_with(&format!("renoir_{service}_rsp_")) && !name.ends_with(".lease")
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let ring_name = path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches("renoir_")
        .to_owned();
    let _servers_cached_mapping = LeanChannel::open(&ring_name, 48, 16, false).unwrap();

    let mut lease = path.as_os_str().to_owned();
    lease.push(".lease");
    while !PathBuf::from(&lease).exists() {
        assert!(std::time::Instant::now() < deadline, "child never leased");
        std::thread::sleep(Duration::from_millis(20));
    }
    for file in [path.clone(), PathBuf::from(lease)] {
        std::fs::OpenOptions::new()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
    }

    let _server = RpcServer::open(&service, 32, 16).unwrap();
    assert!(path.exists(), "the live child's ring was reaped");

    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        path.exists(),
        "a killed client cannot clean up after itself"
    );
    let _server_two = RpcServer::open(&service, 32, 16).unwrap();
    assert!(
        !path.exists(),
        "the kernel released the lock but the ring stayed"
    );
}

#[test]
fn replying_to_a_client_that_has_gone_is_refused_instead_of_recreating_its_ring() {
    let service = unique("gone");
    let mut server = RpcServer::open(&service, 32, 16).unwrap();
    let mut client = RpcClient::open(&service, 32, 16).unwrap();
    let id = client.client_id();
    let mut out = [0u8; 32];
    assert!(client
        .call(b"hi", &mut out, Duration::from_millis(20))
        .is_err());
    let mut buf = [0u8; 32];
    let req = server.poll(&mut buf).unwrap().unwrap();
    drop(client);
    assert!(server.reply(id, req.sequence, b"late").is_err());
    assert_eq!(count_responses(&service), 0);
}
