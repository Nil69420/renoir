use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use renoir::topic_rings::{monotonic_ns, LeanChannel, LeanClock, LeanMode};

const ROLE: &str = "RENOIR_STAMP_ROLE";
const NAME: &str = "RENOIR_STAMP_NAME";
const TEST: &str = "stamps_from_another_process_share_the_monotonic_clock";

fn child() {
    let name = std::env::var(NAME).unwrap();
    let mut ch = LeanChannel::open_stamped(
        &name,
        16,
        64,
        false,
        LeanMode::SingleProducer,
        LeanClock::Nanoseconds,
    )
    .unwrap();
    for i in 0..20u64 {
        ch.publish_now(&[i as u8; 16]).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn stamps_from_another_process_share_the_monotonic_clock() {
    if std::env::var(ROLE).is_ok() {
        child();
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("RENOIR_SHM_DIR", dir.path());
    let name = format!("stamp_proc_{}", std::process::id());
    let mut reader = LeanChannel::open_stamped(
        &name,
        16,
        64,
        true,
        LeanMode::SingleProducer,
        LeanClock::Nanoseconds,
    )
    .unwrap();
    let start = monotonic_ns();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(ROLE, "child")
        .env(NAME, &name)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut stamps = Vec::new();
    let mut out = [0u8; 16];
    let deadline = Instant::now() + Duration::from_secs(20);
    while stamps.len() < 20 && Instant::now() < deadline {
        match reader.try_read_stamped(&mut out).unwrap() {
            Some(s) => {
                assert_eq!(out[0] as usize, stamps.len());
                stamps.push(s);
            }
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    child.wait().unwrap();
    let end = monotonic_ns();
    assert_eq!(stamps.len(), 20);
    assert!(stamps.iter().all(|s| *s >= start && *s <= end));
    assert!(stamps.windows(2).all(|w| w[1] > w[0]));
    let gap = (stamps[19] - stamps[0]) / 19;
    assert!(
        (4_000_000..50_000_000).contains(&gap),
        "mean spacing {gap} ns is not near the 5 ms publish period"
    );
}
