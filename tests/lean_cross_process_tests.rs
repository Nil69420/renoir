use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use renoir::ffi::{
    renoir_lean_close, renoir_lean_lost, renoir_lean_open, renoir_lean_publish,
    renoir_lean_try_read,
};

const ROLE: &str = "RENOIR_LEAN_XPROC_ROLE";
const NAME: &str = "RENOIR_LEAN_XPROC_NAME";
const COUNT: u64 = 200_000;
const MSG: usize = 32;

fn shm_dir() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    });
}

fn open(name: &str, oldest: u32) -> *mut std::ffi::c_void {
    let cname = std::ffi::CString::new(name).unwrap();
    let mut h = std::ptr::null_mut();
    let rc = renoir_lean_open(cname.as_ptr(), MSG as u32, 256, oldest, &mut h);
    assert_eq!(rc as i32, 0, "open failed");
    h
}

fn encode(seq: u64) -> [u8; MSG] {
    let mut b = [0u8; MSG];
    for chunk in b.chunks_exact_mut(8) {
        chunk.copy_from_slice(&seq.to_le_bytes());
    }
    b
}

fn consumer(name: &str) {
    let h = open(name, 1);
    let mut buf = [0u8; MSG];
    let mut last = 0u64;
    let mut received = 0u64;
    let deadline = Instant::now() + Duration::from_secs(60);
    while last < COUNT && Instant::now() < deadline {
        if renoir_lean_try_read(h, buf.as_mut_ptr(), MSG) as i32 != 0 {
            std::hint::spin_loop();
            continue;
        }
        let first = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        for chunk in buf.chunks_exact(8) {
            assert_eq!(
                u64::from_le_bytes(chunk.try_into().unwrap()),
                first,
                "torn read"
            );
        }
        assert!(first > last, "out of order: {first} after {last}");
        last = first;
        received += 1;
    }
    assert_eq!(last, COUNT, "consumer never saw the final message");
    assert_eq!(received + renoir_lean_lost(h), COUNT);
    renoir_lean_close(h);
}

fn spawn(test: &str, role: &str, name: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(ROLE, role)
        .env(NAME, name)
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn publisher_and_consumers_in_separate_processes() {
    if let Ok(role) = std::env::var(ROLE) {
        let name = std::env::var(NAME).unwrap();
        if role == "consumer" {
            consumer(&name);
        }
        return;
    }
    shm_dir();
    let name = format!("lean_xproc_{}", std::process::id());
    let publisher = open(&name, 0);
    let mut children: Vec<_> = (0..3)
        .map(|_| {
            spawn(
                "publisher_and_consumers_in_separate_processes",
                "consumer",
                &name,
            )
        })
        .collect();
    std::thread::sleep(Duration::from_millis(500));
    for seq in 1..=COUNT {
        assert_eq!(
            renoir_lean_publish(publisher, encode(seq).as_ptr(), MSG) as i32,
            0
        );
        if seq % 64 == 0 {
            std::hint::spin_loop();
        }
    }
    for c in &mut children {
        assert!(c.wait().unwrap().success(), "consumer process failed");
    }
    renoir_lean_close(publisher);
}

#[test]
fn simultaneous_first_open_from_many_processes() {
    if let Ok(role) = std::env::var(ROLE) {
        if role == "opener" {
            let name = std::env::var(NAME).unwrap();
            let h = open(&name, 0);
            for seq in 1..=50u64 {
                renoir_lean_publish(h, encode(seq).as_ptr(), MSG);
            }
            let mut buf = [0u8; MSG];
            while renoir_lean_try_read(h, buf.as_mut_ptr(), MSG) as i32 == 0 {}
            renoir_lean_close(h);
        }
        return;
    }
    shm_dir();
    for round in 0..20 {
        let name = format!("lean_race_{}_{round}", std::process::id());
        let mut children: Vec<_> = (0..8)
            .map(|_| {
                spawn(
                    "simultaneous_first_open_from_many_processes",
                    "opener",
                    &name,
                )
            })
            .collect();
        for c in &mut children {
            assert!(
                c.wait().unwrap().success(),
                "opener crashed in round {round}"
            );
        }
    }
}
