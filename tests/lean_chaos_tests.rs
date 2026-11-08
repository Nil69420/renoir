use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use renoir::ffi::{renoir_lean_close, renoir_lean_open, renoir_lean_publish, renoir_lean_try_read};

const ROLE: &str = "RENOIR_LEAN_CHAOS_ROLE";
const NAME: &str = "RENOIR_LEAN_CHAOS_NAME";
const GEN: &str = "RENOIR_LEAN_CHAOS_GEN";
const LIMIT: &str = "RENOIR_LEAN_CHAOS_LIMIT";
const MSG: usize = 32;
const ROUNDS: u64 = 150;
const FINAL_COUNT: u64 = 2000;

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

fn encode(value: u64) -> [u8; MSG] {
    let mut b = [0u8; MSG];
    for chunk in b.chunks_exact_mut(8) {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
    b
}

fn publisher_child(name: &str, generation: u64, limit: u64) {
    let h = open(name, 0);
    let mut n = 0u64;
    while n < limit {
        n += 1;
        let value = (generation << 40) | n;
        assert_eq!(
            renoir_lean_publish(h, encode(value).as_ptr(), MSG) as i32,
            0
        );
    }
    renoir_lean_close(h);
}

fn spawn(name: &str, generation: u64, limit: u64) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "publisher_killed_repeatedly_never_corrupts_readers",
            "--nocapture",
        ])
        .env(ROLE, "publisher")
        .env(NAME, name)
        .env(GEN, generation.to_string())
        .env(LIMIT, limit.to_string())
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn publisher_killed_repeatedly_never_corrupts_readers() {
    if std::env::var(ROLE).is_ok() {
        let name = std::env::var(NAME).unwrap();
        let generation: u64 = std::env::var(GEN).unwrap().parse().unwrap();
        let limit: u64 = std::env::var(LIMIT).unwrap().parse().unwrap();
        publisher_child(&name, generation, limit);
        return;
    }
    shm_dir();
    let name = format!("lean_chaos_{}", std::process::id());
    let reader = open(&name, 1);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = stop.clone();
    let seen_generation = Arc::new(AtomicU64::new(u64::MAX));
    let seen_reader = seen_generation.clone();
    let reader_addr = reader as usize;
    let thread = std::thread::spawn(move || {
        let h = reader_addr as *mut std::ffi::c_void;
        let mut buf = [0u8; MSG];
        let mut last = 0u64;
        let mut generations = std::collections::BTreeSet::new();
        while !stop_reader.load(Ordering::Relaxed) {
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
            generations.insert(first >> 40);
            seen_reader.store(first >> 40, Ordering::Release);
        }
        (last, generations)
    });

    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    for generation in 0..ROUNDS {
        let mut child = spawn(&name, generation, u64::MAX);
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let deadline = Instant::now() + Duration::from_secs(30);
        while seen_generation.load(Ordering::Acquire) != generation && Instant::now() < deadline {
            std::thread::sleep(Duration::from_micros(100));
        }
        std::thread::sleep(Duration::from_micros(rng % 5_000));
        child.kill().unwrap();
        child.wait().unwrap();
    }

    let mut survivor = spawn(&name, ROUNDS, FINAL_COUNT);
    assert!(
        survivor.wait().unwrap().success(),
        "post-chaos publisher failed"
    );
    let expected = (ROUNDS << 40) | FINAL_COUNT;
    std::thread::sleep(Duration::from_millis(200));
    stop.store(true, Ordering::Relaxed);
    let (final_last, generations) = thread.join().unwrap();
    assert!(
        generations.len() as u64 >= ROUNDS * 9 / 10,
        "reader saw only {} of {ROUNDS} publisher generations",
        generations.len()
    );
    assert_eq!(
        final_last, expected,
        "reader did not recover after publisher kills"
    );
    renoir_lean_close(reader);
}
