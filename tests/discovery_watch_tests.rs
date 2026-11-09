use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use renoir::discovery::{DiscoveryEvent, DiscoveryWatcher};
use renoir::topic_rings::{LeanChannel, LeanMode};

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ROLE: &str = "RENOIR_WATCH_ROLE";
const NAME: &str = "RENOIR_WATCH_NAME";

fn dir() -> &'static std::path::Path {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    })
    .path()
}

fn publisher_child() {
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
    let name = std::env::var(NAME).unwrap();
    let mut channel =
        LeanChannel::open_with_mode(&name, 24, 16, false, LeanMode::MultiProducerGated).unwrap();
    channel.publish(&[1u8; 24]).unwrap();
    loop {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn next_event(
    watcher: &mut DiscoveryWatcher,
    within: Duration,
    mut wanted: impl FnMut(&DiscoveryEvent) -> bool,
) -> Option<(DiscoveryEvent, Duration)> {
    let start = Instant::now();
    while start.elapsed() < within {
        for event in watcher.poll(within.saturating_sub(start.elapsed())) {
            if wanted(&event) {
                return Some((event, start.elapsed()));
            }
        }
    }
    None
}

#[test]
fn watcher_reports_appear_change_dead_publisher_and_disappear() {
    if std::env::var(ROLE).is_ok() {
        publisher_child();
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = dir();
    let name = format!("watch_{}", std::process::id());
    let mut watcher = DiscoveryWatcher::open_dir(dir, Duration::from_millis(20)).unwrap();
    assert!(watcher.snapshot().is_empty());
    assert!(watcher.poll(Duration::from_millis(30)).is_empty());

    let mut channel =
        LeanChannel::open_with_mode(&name, 24, 16, false, LeanMode::MultiProducerGated).unwrap();
    channel.publish(&[9u8; 24]).unwrap();
    let (event, took) = next_event(
        &mut watcher,
        Duration::from_secs(2),
        |e| matches!(e, DiscoveryEvent::Appeared(info) if info.name == name),
    )
    .expect("ring appearance not reported");
    assert!(
        took < Duration::from_millis(500),
        "appearance took {took:?}"
    );
    let DiscoveryEvent::Appeared(info) = event else {
        unreachable!()
    };
    assert_eq!(info.lean.as_ref().unwrap().live_publishers, 1);
    assert_eq!(watcher.snapshot().len(), 1);

    channel.register_consumer(false).unwrap();
    let (event, _) = next_event(
        &mut watcher,
        Duration::from_secs(2),
        |e| matches!(e, DiscoveryEvent::Changed { current, .. } if current.name == name),
    )
    .expect("consumer registration not reported");
    let DiscoveryEvent::Changed { previous, current } = event else {
        unreachable!()
    };
    assert_eq!(previous.lean.as_ref().unwrap().registered_consumers, 0);
    assert_eq!(current.lean.as_ref().unwrap().registered_consumers, 1);

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "watcher_reports_appear_change_dead_publisher_and_disappear",
            "--nocapture",
        ])
        .env(ROLE, "child")
        .env(NAME, &name)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let (event, _) = next_event(&mut watcher, Duration::from_secs(5), |e| {
        matches!(e, DiscoveryEvent::Changed { current, .. }
            if current.lean.as_ref().unwrap().live_publishers == 2)
    })
    .expect("second publisher not reported");
    assert!(matches!(event, DiscoveryEvent::Changed { .. }));
    child.kill().unwrap();
    child.wait().unwrap();
    let (event, _) = next_event(&mut watcher, Duration::from_secs(5), |e| {
        matches!(e, DiscoveryEvent::Changed { current, .. }
            if current.lean.as_ref().unwrap().live_publishers == 1)
    })
    .expect("publisher death not reported");
    assert!(matches!(event, DiscoveryEvent::Changed { .. }));

    drop(channel);
    std::fs::remove_file(dir.join(format!("renoir_{name}"))).unwrap();
    let (_, took) = next_event(
        &mut watcher,
        Duration::from_secs(2),
        |e| matches!(e, DiscoveryEvent::Disappeared { name: gone } if *gone == name),
    )
    .expect("ring removal not reported");
    assert!(took < Duration::from_millis(500), "removal took {took:?}");
    assert!(watcher.snapshot().is_empty());
}

#[test]
fn watcher_with_a_waiting_poll_is_woken_early_by_a_new_ring() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = dir();
    let name = format!("wake_{}", std::process::id());
    let mut watcher = DiscoveryWatcher::open_dir(dir, Duration::from_secs(30)).unwrap();
    let handle = {
        let name = name.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let mut channel =
                LeanChannel::open_with_mode(&name, 24, 16, false, LeanMode::SingleProducer)
                    .unwrap();
            channel.publish(&[1u8; 24]).unwrap();
            std::thread::sleep(Duration::from_millis(1500));
        })
    };
    let (_, took) = next_event(
        &mut watcher,
        Duration::from_secs(10),
        |e| matches!(e, DiscoveryEvent::Appeared(i) if i.name == name),
    )
    .expect("no appearance reported");
    assert!(
        took < Duration::from_secs(1),
        "a 30 s interval watcher woke after {took:?}"
    );
    handle.join().unwrap();
    let _ = std::fs::remove_file(dir.join(format!("renoir_{name}")));
}

#[test]
fn a_ring_initialised_through_mmap_is_found_by_the_fast_rescan() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = dir();
    let source = format!("src_{}", std::process::id());
    let target = format!("late_{}", std::process::id());
    let channel =
        LeanChannel::open_with_mode(&source, 24, 16, false, LeanMode::SingleProducer).unwrap();
    let header = std::fs::read(dir.join(format!("renoir_{source}"))).unwrap();
    let target_path = dir.join(format!("renoir_{target}"));
    std::fs::write(&target_path, vec![0u8; header.len()]).unwrap();

    let mut watcher = DiscoveryWatcher::open_dir(dir, Duration::from_secs(30)).unwrap();
    let handle = {
        let target_path = target_path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&target_path)
                .unwrap();
            let mut map = unsafe { memmap2::MmapMut::map_mut(&file).unwrap() };
            map.copy_from_slice(&header);
            map.flush().unwrap();
        })
    };
    let (_, took) = next_event(
        &mut watcher,
        Duration::from_secs(5),
        |e| matches!(e, DiscoveryEvent::Appeared(i) if i.name == target),
    )
    .expect("late initialisation not reported");
    assert!(took < Duration::from_secs(1), "found after {took:?}");
    handle.join().unwrap();
    drop(channel);
    let _ = std::fs::remove_file(dir.join(format!("renoir_{source}")));
    let _ = std::fs::remove_file(target_path);
}
