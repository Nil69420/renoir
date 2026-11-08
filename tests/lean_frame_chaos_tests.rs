use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use renoir::topic_rings::FrameChannel;

const ROLE: &str = "RENOIR_FRAME_CHAOS_ROLE";
const NAME: &str = "RENOIR_FRAME_CHAOS_NAME";
const MAX: usize = 4096;
const SLOTS: usize = 4;

fn shm_dir() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    });
}

fn shm_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("RENOIR_SHM_DIR").unwrap())
}

fn die_with_parent() {
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
}

fn holder_child() {
    die_with_parent();
    let name = std::env::var(NAME).unwrap();
    let mut ch = FrameChannel::open(&name, MAX, SLOTS, false, true, true).unwrap();
    loop {
        if ch.view_raw().unwrap().is_some() {
            loop {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn loaner_child() {
    die_with_parent();
    let name = std::env::var(NAME).unwrap();
    let mut ch = FrameChannel::open(&name, MAX, SLOTS, false, false, false).unwrap();
    let ptr = ch.loan_raw(100).unwrap();
    unsafe { std::ptr::write_bytes(ptr, 0xEE, 100) };
    loop {
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn spawn(role: &str, name: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "frame_holders_and_loaners_that_die_never_wedge_the_ring",
            "--nocapture",
        ])
        .env(ROLE, role)
        .env(NAME, name)
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn frame_holders_and_loaners_that_die_never_wedge_the_ring() {
    match std::env::var(ROLE).as_deref() {
        Ok("holder") => holder_child(),
        Ok("loaner") => loaner_child(),
        _ => {}
    }
    shm_dir();
    let name = format!("frame_chaos_{}", std::process::id());
    let mut producer = FrameChannel::open(&name, MAX, SLOTS, false, false, false).unwrap();

    let mut holder = spawn("holder", &name);
    let deadline = Instant::now() + Duration::from_secs(30);
    let dir = shm_path();
    loop {
        let registered = renoir::discovery::scan_dir(&dir)
            .iter()
            .filter_map(|r| r.lean.as_ref())
            .map(|l| l.registered_consumers)
            .max()
            .unwrap_or(0);
        if registered >= 1 {
            break;
        }
        assert!(Instant::now() < deadline, "child consumer never registered");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut published = 0u32;
    loop {
        assert!(Instant::now() < deadline, "producer never back-pressured");
        match producer.loan(64) {
            Ok(mut loan) => {
                loan.bytes_mut().fill(published as u8);
                loan.commit().unwrap();
                published += 1;
            }
            Err(_) => break,
        }
        std::thread::sleep(Duration::from_millis(3));
        if published > 64 {
            break;
        }
    }
    assert!(published >= 1);
    assert!(
        producer.loan(64).is_err(),
        "a live consumer holding a frame must back-pressure the producer"
    );
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        producer.loan(64).is_err(),
        "a live holder must not be evicted by the timer"
    );

    holder.kill().unwrap();
    holder.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match producer.loan(64) {
            Ok(loan) => {
                loan.abort();
                break;
            }
            Err(_) => {
                assert!(Instant::now() < deadline, "dead holder was never reclaimed");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    let mut loaner = spawn("loaner", &name);
    std::thread::sleep(Duration::from_millis(400));
    loaner.kill().unwrap();
    loaner.wait().unwrap();
    let mut consumer = FrameChannel::open(&name, MAX, SLOTS, false, true, false).unwrap();
    let mut ok = false;
    for _ in 0..200 {
        if let Ok(mut loan) = producer.loan(10) {
            loan.bytes_mut().fill(0x5A);
            loan.commit().unwrap();
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ok, "producer wedged after a loaner died");
    let view = consumer.view().unwrap();
    if let Some(v) = view {
        assert_eq!(v.len(), 10);
        assert!(v.bytes().iter().all(|b| *b == 0x5A));
    } else {
        panic!("frame not visible to the new consumer");
    }
}
