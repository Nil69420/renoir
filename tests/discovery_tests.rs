use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use renoir::discovery::{render_table, scan_dir, sweep_dir, RingKind, SweepOptions};
use renoir::topic_rings::{LeanChannel, LeanMode};

const ROLE: &str = "RENOIR_DISCOVERY_ROLE";
const NAME: &str = "RENOIR_DISCOVERY_NAME";

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
    let mut ch =
        LeanChannel::open_with_mode(&name, 24, 16, false, LeanMode::MultiProducerGated).unwrap();
    ch.publish(&[1u8; 24]).unwrap();
    loop {
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn discovery_describes_sweeps_and_notices_dead_publishers() {
    if std::env::var(ROLE).is_ok() {
        publisher_child();
        return;
    }
    let dir = dir();
    std::fs::write(dir.join("renoir_not_a_ring"), vec![7u8; 4096]).unwrap();
    std::fs::write(dir.join("unrelated"), b"x").unwrap();
    let name = format!("disc_{}", std::process::id());

    let mut channel =
        LeanChannel::open_with_mode(&name, 24, 16, false, LeanMode::MultiProducerGated).unwrap();
    channel.register_consumer(false).unwrap();
    channel.publish(&[9u8; 24]).unwrap();

    let found = scan_dir(dir);
    assert_eq!(found.len(), 1, "only real rings are listed: {found:?}");
    let ring = &found[0];
    assert_eq!(ring.name, name);
    assert_eq!(ring.kind, RingKind::Lean);
    assert!(ring.mapped_by.contains(&std::process::id()));
    let lean = ring.lean.as_ref().unwrap();
    assert_eq!(lean.msg_size, 24);
    assert_eq!(lean.slot_count, 16);
    assert_eq!(lean.mode, LeanMode::MultiProducerGated);
    assert_eq!(lean.published, 1);
    assert_eq!(lean.live_publishers, 1);
    assert_eq!(lean.registered_consumers, 1);

    let text = render_table(&found, &std::collections::HashMap::new(), Instant::now());
    let row = text
        .lines()
        .find(|l| l.starts_with(&name))
        .unwrap_or_else(|| panic!("ring missing from table:\n{text}"));
    assert!(row.contains("lean") && row.contains("mp-gated"), "{row}");

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "discovery_describes_sweeps_and_notices_dead_publishers",
            "--nocapture",
        ])
        .env(ROLE, "child")
        .env(NAME, &name)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let live = scan_dir(dir)[0].lean.as_ref().unwrap().live_publishers;
        if live == 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child publisher never appeared ({live})"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(scan_dir(dir)[0].lean.as_ref().unwrap().live_publishers, 1);

    let force = SweepOptions {
        dry_run: false,
        ignore_unreadable_processes: true,
    };
    let report = sweep_dir(dir, force);
    assert!(report.removed.is_empty());
    assert_eq!(report.kept_in_use.len(), 1);

    drop(channel);
    assert!(scan_dir(dir)[0].mapped_by.is_empty());
    let dry = sweep_dir(
        dir,
        SweepOptions {
            dry_run: true,
            ignore_unreadable_processes: true,
        },
    );
    assert_eq!(dry.removed.len(), 1);
    assert_eq!(scan_dir(dir).len(), 1);
    let real = sweep_dir(dir, force);
    assert_eq!(real.removed.len(), 1);
    assert!(scan_dir(dir).is_empty());
    assert!(dir.join("renoir_not_a_ring").exists());
    assert!(dir.join("unrelated").exists());
}
