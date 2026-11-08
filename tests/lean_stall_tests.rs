use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use renoir::ffi::{
    renoir_lean_close, renoir_lean_lost, renoir_lean_open, renoir_lean_publish,
    renoir_lean_try_read, RENOIR_LEAN_CONSUMER, RENOIR_LEAN_FROM_OLDEST, RENOIR_LEAN_GATED,
    RENOIR_LEAN_MULTI_PRODUCER,
};

const ROLE: &str = "RENOIR_LEAN_STALL_ROLE";
const NAME: &str = "RENOIR_LEAN_STALL_NAME";
const ID: &str = "RENOIR_LEAN_STALL_ID";
const MSG: usize = 16;
const SLOTS: u32 = 16;
const PRODUCERS: u64 = 3;
const PER: u64 = 30_000;
const STOPS: u32 = 12;

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
        flags | RENOIR_LEAN_GATED | RENOIR_LEAN_MULTI_PRODUCER,
        &mut h,
    );
    assert_eq!(rc as i32, 0, "open failed");
    h
}

fn encode(id: u64, n: u64) -> [u8; MSG] {
    let mut b = [0u8; MSG];
    b[..8].copy_from_slice(&id.to_le_bytes());
    b[8..].copy_from_slice(&n.to_le_bytes());
    b
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
    let h = open(&name, 0);
    for n in 1..=PER {
        let data = encode(id, n);
        while renoir_lean_publish(h, data.as_ptr(), MSG) as i32 != 0 {
            std::hint::spin_loop();
        }
    }
    renoir_lean_close(h);
}

#[test]
fn stopped_but_alive_producers_never_lose_messages() {
    if std::env::var(ROLE).is_ok() {
        publisher_child();
        return;
    }
    shm_dir();
    let name = format!("lean_stall_{}", std::process::id());
    let consumer = open(&name, RENOIR_LEAN_CONSUMER | RENOIR_LEAN_FROM_OLDEST) as usize;
    let mut children: Vec<std::process::Child> = (0..PRODUCERS)
        .map(|id| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "stopped_but_alive_producers_never_lose_messages",
                    "--nocapture",
                ])
                .env(ROLE, "publisher")
                .env(NAME, &name)
                .env(ID, id.to_string())
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();

    let reader = std::thread::spawn(move || {
        let h = consumer as *mut std::ffi::c_void;
        let mut buf = [0u8; MSG];
        let mut last = [0u64; PRODUCERS as usize];
        let mut total = 0u64;
        let deadline = Instant::now() + Duration::from_secs(180);
        while total < PRODUCERS * PER {
            assert!(Instant::now() < deadline, "consumer starved at {total}");
            if renoir_lean_try_read(h, buf.as_mut_ptr(), MSG) as i32 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let id = u64::from_le_bytes(buf[..8].try_into().unwrap()) as usize;
            let n = u64::from_le_bytes(buf[8..].try_into().unwrap());
            assert!(id < last.len(), "corrupt producer id");
            assert_eq!(n, last[id] + 1, "producer {id} lost or reordered a message");
            last[id] = n;
            total += 1;
        }
        (total, renoir_lean_lost(h))
    });

    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    for round in 0..STOPS {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let victim = (rng % PRODUCERS) as usize;
        let pid = children[victim].id() as i32;
        std::thread::sleep(Duration::from_millis(10 + rng % 30));
        unsafe { libc::kill(pid, libc::SIGSTOP) };
        std::thread::sleep(Duration::from_millis(200 + u64::from(round) * 5));
        unsafe { libc::kill(pid, libc::SIGCONT) };
    }
    for child in &mut children {
        unsafe { libc::kill(child.id() as i32, libc::SIGCONT) };
        assert!(child.wait().unwrap().success());
    }
    let (total, lost) = reader.join().unwrap();
    assert_eq!(total, PRODUCERS * PER);
    assert_eq!(
        lost, 0,
        "messages were skipped while a producer was merely stopped"
    );
    renoir_lean_close(consumer as *mut std::ffi::c_void);
}
