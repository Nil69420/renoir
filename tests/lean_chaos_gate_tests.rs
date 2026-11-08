use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use renoir::ffi::{
    renoir_lean_close, renoir_lean_open, renoir_lean_publish, renoir_lean_try_read,
    RENOIR_LEAN_CONSUMER, RENOIR_LEAN_GATED,
};
use renoir::topic_rings::{LeanChannel, LeanMode};

const ROLE: &str = "RENOIR_LEAN_CHAOS_GATE_ROLE";
const NAME: &str = "RENOIR_LEAN_CHAOS_GATE_NAME";
const MSG: usize = 16;
const SLOTS: u32 = 32;

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
        flags | RENOIR_LEAN_GATED,
        &mut h,
    );
    assert_eq!(rc as i32, 0, "open failed");
    h
}

fn encode(n: u64) -> [u8; MSG] {
    let mut b = [0u8; MSG];
    b[..8].copy_from_slice(&n.to_le_bytes());
    b[8..].copy_from_slice(&n.to_le_bytes());
    b
}

fn slow_consumer_child() {
    let name = std::env::var(NAME).unwrap();
    let h = open(&name, RENOIR_LEAN_CONSUMER);
    let mut buf = [0u8; MSG];
    loop {
        let _ = renoir_lean_try_read(h, buf.as_mut_ptr(), MSG);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn publisher_survives_killed_registered_consumer() {
    if std::env::var(ROLE).is_ok() {
        slow_consumer_child();
        return;
    }
    shm_dir();
    let name = format!("lean_chaos_gate_{}", std::process::id());
    let publisher = open(&name, 0);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "publisher_survives_killed_registered_consumer",
            "--nocapture",
        ])
        .env(ROLE, "consumer")
        .env(NAME, &name)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let mut n = 0u64;
    let started = Instant::now();
    while n < 200 {
        n += 1;
        while renoir_lean_publish(publisher, encode(n).as_ptr(), MSG) as i32 != 0 {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "stalled with live consumer"
            );
            std::hint::spin_loop();
        }
    }
    child.kill().unwrap();
    child.wait().unwrap();

    let killed_at = Instant::now();
    while n < 5_000 {
        n += 1;
        while renoir_lean_publish(publisher, encode(n).as_ptr(), MSG) as i32 != 0 {
            assert!(
                killed_at.elapsed() < Duration::from_secs(10),
                "publisher wedged by a dead consumer"
            );
            std::hint::spin_loop();
        }
    }

    let mut replacement = LeanChannel::open_with_mode(
        &name,
        MSG,
        SLOTS as usize,
        false,
        LeanMode::SingleProducerGated,
    )
    .unwrap();
    replacement.register_consumer(false).unwrap();
    for k in 1..=20u64 {
        n += 1;
        let _ = k;
        while renoir_lean_publish(publisher, encode(n).as_ptr(), MSG) as i32 != 0 {
            let mut out = [0u8; MSG];
            let _ = replacement.try_read(&mut out);
        }
    }
    let mut out = [0u8; MSG];
    let mut last = 0u64;
    let mut seen = 0;
    while replacement.try_read(&mut out).unwrap() == renoir::topic_rings::LeanRead::Message {
        let v = u64::from_le_bytes(out[..8].try_into().unwrap());
        assert!(v > last);
        last = v;
        seen += 1;
    }
    assert!(seen > 0);
    renoir_lean_close(publisher);
}
