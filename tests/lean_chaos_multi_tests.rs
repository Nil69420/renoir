use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use renoir::ffi::{
    renoir_lean_close, renoir_lean_lost, renoir_lean_open, renoir_lean_publish,
    renoir_lean_try_read, RENOIR_LEAN_FROM_OLDEST, RENOIR_LEAN_MULTI_PRODUCER,
};

const ROLE: &str = "RENOIR_LEAN_CHAOS_MP_ROLE";
const NAME: &str = "RENOIR_LEAN_CHAOS_MP_NAME";
const ID: &str = "RENOIR_LEAN_CHAOS_MP_ID";
const GEN: &str = "RENOIR_LEAN_CHAOS_MP_GEN";
const LIMIT: &str = "RENOIR_LEAN_CHAOS_MP_LIMIT";
const MSG: usize = 32;
const SLOTS: u32 = 16;
const PRODUCERS: u64 = 3;
const KILLS: u64 = 300;
const FINAL_COUNT: u64 = 500;

fn shm_dir() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    });
}

fn open(name: &str, flags: u32) -> *mut std::ffi::c_void {
    let cname = std::ffi::CString::new(name).unwrap();
    let mut h = std::ptr::null_mut();
    let rc = renoir_lean_open(
        cname.as_ptr(),
        MSG as u32,
        SLOTS,
        flags | RENOIR_LEAN_MULTI_PRODUCER,
        &mut h,
    );
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

fn spawn(name: &str, id: u64, generation: u64, limit: u64) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "killed_producers_never_corrupt_or_wedge_the_ring",
            "--nocapture",
        ])
        .env(ROLE, "publisher")
        .env(NAME, name)
        .env(ID, id.to_string())
        .env(GEN, generation.to_string())
        .env(LIMIT, limit.to_string())
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

fn die_with_parent() {
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
}

fn publisher_child() {
    die_with_parent();
    let name = std::env::var(NAME).unwrap();
    let id: u64 = std::env::var(ID).unwrap().parse().unwrap();
    let generation: u64 = std::env::var(GEN).unwrap().parse().unwrap();
    let limit: u64 = std::env::var(LIMIT).unwrap().parse().unwrap();
    let h = open(&name, 0);
    let mut n = 0u64;
    while n < limit {
        n += 1;
        let value = (id << 56) | (generation << 40) | n;
        let _ = renoir_lean_publish(h, encode(value).as_ptr(), MSG);
    }
    renoir_lean_close(h);
}

#[test]
fn killed_producers_never_corrupt_or_wedge_the_ring() {
    if std::env::var(ROLE).is_ok() {
        publisher_child();
        return;
    }
    shm_dir();
    let name = format!("lean_chaos_mp_{}", std::process::id());
    let reader = open(&name, RENOIR_LEAN_FROM_OLDEST) as usize;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = stop.clone();
    let thread = std::thread::spawn(move || {
        let h = reader as *mut std::ffi::c_void;
        let mut buf = [0u8; MSG];
        let mut last = [0u64; PRODUCERS as usize + 1];
        let mut total = 0u64;
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
            let id = (first >> 56) as usize;
            assert!(id < last.len(), "corrupt producer id");
            let tail = first & ((1u64 << 56) - 1);
            assert!(tail > last[id], "producer {id} went backwards");
            last[id] = tail;
            total += 1;
        }
        (last, total, renoir_lean_lost(h))
    });

    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut children: Vec<Option<std::process::Child>> = (0..PRODUCERS).map(|_| None).collect();
    for round in 0..KILLS {
        let id = round % PRODUCERS;
        if let Some(mut old) = children[id as usize].take() {
            old.kill().unwrap();
            old.wait().unwrap();
        }
        children[id as usize] = Some(spawn(&name, id, round, u64::MAX));
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        std::thread::sleep(Duration::from_micros(500 + rng % 8_000));
    }
    for child in children.iter_mut().flatten() {
        child.kill().unwrap();
        child.wait().unwrap();
    }

    let mut survivors: Vec<_> = (0..PRODUCERS)
        .map(|id| spawn(&name, id, KILLS + 1, FINAL_COUNT))
        .collect();
    for s in &mut survivors {
        assert!(s.wait().unwrap().success(), "post-chaos publisher failed");
    }
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    let (last, total, lost) = thread.join().unwrap();
    for (id, &seen) in last.iter().enumerate().take(PRODUCERS as usize) {
        assert_eq!(
            seen,
            ((KILLS + 1) << 40) | FINAL_COUNT,
            "ring wedged: producer {id} final message never arrived (lost {lost}, read {total})"
        );
    }
    renoir_lean_close(reader as *mut std::ffi::c_void);
}
