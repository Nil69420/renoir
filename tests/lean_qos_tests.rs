use std::time::Duration;

use renoir::topic_rings::{DeadlineWait, LeanChannel, LeanQos, LeanRead};

fn shm_dir() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    });
}

fn unique(tag: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "qos_{tag}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn best_effort_overwrites_old_messages() {
    shm_dir();
    let name = unique("be");
    let qos = LeanQos::best_effort(4);
    let mut publisher = LeanChannel::open_qos(&name, 8, qos, false, false).unwrap();
    let mut subscriber = LeanChannel::open_qos(&name, 8, qos, false, true).unwrap();
    for i in 0u64..10 {
        publisher.publish(&i.to_le_bytes()).unwrap();
    }
    let mut out = [0u8; 8];
    let mut got = Vec::new();
    while subscriber.try_read(&mut out).unwrap() == LeanRead::Message {
        got.push(u64::from_le_bytes(out));
    }
    assert!(got.len() <= 4);
    assert_eq!(*got.last().unwrap(), 9);
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn reliable_never_overwrites_unread_messages() {
    shm_dir();
    let name = unique("rel");
    let qos = LeanQos::reliable(4);
    let mut publisher = LeanChannel::open_qos(&name, 8, qos, false, false).unwrap();
    let mut subscriber = LeanChannel::open_qos(&name, 8, qos, false, true).unwrap();
    for i in 0u64..4 {
        publisher.publish(&i.to_le_bytes()).unwrap();
    }
    assert!(publisher.publish(&4u64.to_le_bytes()).is_err());
    let mut out = [0u8; 8];
    for i in 0u64..4 {
        assert_eq!(subscriber.try_read(&mut out).unwrap(), LeanRead::Message);
        assert_eq!(u64::from_le_bytes(out), i);
    }
    publisher.publish(&4u64.to_le_bytes()).unwrap();
    assert_eq!(subscriber.lost(), 0);
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn depth_below_two_is_rejected() {
    shm_dir();
    assert!(
        LeanChannel::open_qos(&unique("depth"), 8, LeanQos::best_effort(1), false, false).is_err()
    );
}

#[test]
#[cfg_attr(miri, ignore = "Miri has no file-backed mmap")]
fn deadline_miss_is_reported_and_counted() {
    shm_dir();
    let name = unique("deadline");
    let qos = LeanQos::best_effort(8).with_deadline(Duration::from_millis(20));
    let mut publisher = LeanChannel::open_qos(&name, 8, qos, false, false).unwrap();
    let mut subscriber = LeanChannel::open_qos(&name, 8, qos, false, true).unwrap();
    assert_eq!(subscriber.wait_deadline(10), DeadlineWait::Missed);
    assert_eq!(subscriber.wait_deadline(10), DeadlineWait::Missed);
    assert_eq!(subscriber.deadline_misses(), 2);
    publisher.publish(&1u64.to_le_bytes()).unwrap();
    assert_eq!(subscriber.wait_deadline(10), DeadlineWait::Ready);
    assert_eq!(subscriber.deadline_misses(), 2);
}
