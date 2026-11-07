use std::process::Command;
use std::time::{Duration, Instant};

use renoir::topic::MessagePayload;
use renoir::{Message, TopicConfig, TopicManager, TopicPattern};

const CHILD_ENV: &str = "RENOIR_BCAST_CHILD";
const TOPIC_ENV: &str = "RENOIR_BCAST_TOPIC";
const READY_ENV: &str = "RENOIR_BCAST_READY";
const CROSS_TOTAL: u64 = 500;

fn config(name: &str, ring_kib: usize, max_payload: usize) -> TopicConfig {
    TopicConfig {
        name: name.to_string(),
        pattern: TopicPattern::SPMC,
        ring_capacity: ring_kib,
        max_payload_size: max_payload,
        ..Default::default()
    }
}

fn unique(prefix: &str) -> String {
    format!("{}_{}", prefix, std::process::id())
}

fn payload_of(m: &Message) -> &Vec<u8> {
    match &m.payload {
        MessagePayload::Inline(d) => d,
        _ => panic!("expected inline payload"),
    }
}

#[test]
fn test_spmc_two_subscribers_receive_every_camera_frame() {
    let name = unique("bcast_frames");
    let manager = TopicManager::new().unwrap();
    manager.create_topic(config(&name, 32768, 921_600)).unwrap();
    let publisher = manager.create_publisher(&name).unwrap();
    let a = manager.create_subscriber(&name).unwrap();
    let b = manager.create_subscriber(&name).unwrap();

    let mut got_a = 0;
    let mut got_b = 0;
    for i in 0..90u64 {
        publisher.publish(vec![(i % 251) as u8; 921_600]).unwrap();
        while let Some(m) = a.subscribe().unwrap() {
            assert_eq!(payload_of(&m).len(), 921_600);
            got_a += 1;
        }
        while let Some(m) = b.subscribe().unwrap() {
            assert_eq!(payload_of(&m).len(), 921_600);
            got_b += 1;
        }
    }
    assert_eq!(got_a, 90);
    assert_eq!(got_b, 90);
    assert_eq!(a.overruns(), 0);
    assert_eq!(b.overruns(), 0);
}

#[test]
fn test_spmc_slow_subscriber_is_overrun_without_blocking_publisher() {
    let name = unique("bcast_slow");
    let manager = TopicManager::new().unwrap();
    manager.create_topic(config(&name, 64, 1024)).unwrap();
    let publisher = manager.create_publisher(&name).unwrap();
    let fast = manager.create_subscriber(&name).unwrap();
    let slow = manager.create_subscriber(&name).unwrap();

    let total = 1000u64;
    let mut fast_got = 0u64;
    for i in 0..total {
        publisher.publish(vec![(i % 251) as u8; 512]).unwrap();
        while fast.subscribe().unwrap().is_some() {
            fast_got += 1;
        }
    }
    assert_eq!(fast_got, total);
    assert_eq!(fast.overruns(), 0);

    let mut slow_got = 0u64;
    while slow.subscribe().unwrap().is_some() {
        slow_got += 1;
    }
    assert!(slow.overruns() > 0);
    assert_eq!(slow_got + slow.overruns(), total);
}

#[test]
fn test_spmc_late_subscriber_sees_only_new_messages() {
    let name = unique("bcast_late");
    let manager = TopicManager::new().unwrap();
    manager.create_topic(config(&name, 64, 256)).unwrap();
    let publisher = manager.create_publisher(&name).unwrap();
    for _ in 0..5 {
        publisher.publish(vec![1u8; 16]).unwrap();
    }
    let late = manager.create_subscriber(&name).unwrap();
    assert!(late.subscribe().unwrap().is_none());
    publisher.publish(vec![2u8; 16]).unwrap();
    let m = late.subscribe().unwrap().unwrap();
    assert_eq!(payload_of(&m), &vec![2u8; 16]);
}

#[test]
fn test_spmc_rejects_shared_pool_and_oversize_payloads() {
    let name = unique("bcast_reject");
    let manager = TopicManager::new().unwrap();
    let mut bad = config(&name, 64, 256);
    bad.use_shared_pool = true;
    assert!(manager.create_topic(bad).is_err());

    let ok_name = unique("bcast_oversize");
    manager.create_topic(config(&ok_name, 64, 256)).unwrap();
    let publisher = manager.create_publisher(&ok_name).unwrap();
    assert!(publisher.publish(vec![0u8; 100_000]).is_err());
}

#[test]
fn test_spmc_second_publisher_rejected() {
    let name = unique("bcast_onepub");
    let manager = TopicManager::new().unwrap();
    manager.create_topic(config(&name, 64, 256)).unwrap();
    let _first = manager.create_publisher(&name).unwrap();
    assert!(manager.create_publisher(&name).is_err());
}

#[test]
fn test_spmc_cross_process_two_consumers_each_get_everything() {
    if std::env::var(CHILD_ENV).is_ok() {
        run_child();
        return;
    }
    let name = unique("bcast_xproc");
    let manager = TopicManager::new().unwrap();
    manager.create_topic(config(&name, 1024, 4096)).unwrap();
    let publisher = manager.create_publisher(&name).unwrap();

    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    let mut ready_files = Vec::new();
    for i in 0..2 {
        let ready = std::env::temp_dir().join(format!("renoir_bcast_ready_{}_{}", name, i));
        let _ = std::fs::remove_file(&ready);
        let child = Command::new(&exe)
            .args([
                "--exact",
                "test_spmc_cross_process_two_consumers_each_get_everything",
            ])
            .env(CHILD_ENV, "1")
            .env(TOPIC_ENV, &name)
            .env(READY_ENV, &ready)
            .spawn()
            .expect("failed to spawn consumer");
        children.push(child);
        ready_files.push(ready);
    }

    let deadline = Instant::now() + Duration::from_secs(20);
    while ready_files.iter().any(|f| !f.exists()) {
        assert!(Instant::now() < deadline, "consumers never became ready");
        std::thread::sleep(Duration::from_millis(10));
    }

    for i in 1..=CROSS_TOTAL {
        publisher.publish(vec![(i % 251) as u8; 100]).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }

    for mut child in children {
        let status = child.wait().unwrap();
        assert!(status.success(), "consumer failed: {}", status);
    }
    for f in ready_files {
        let _ = std::fs::remove_file(f);
    }
}

fn run_child() {
    let name = std::env::var(TOPIC_ENV).unwrap();
    let ready = std::env::var(READY_ENV).unwrap();
    let manager = TopicManager::new().unwrap();
    manager.create_topic(config(&name, 1024, 4096)).unwrap();
    let subscriber = manager.create_subscriber(&name).unwrap();
    std::fs::write(&ready, b"1").unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut received = 0u64;
    while received < CROSS_TOTAL {
        assert!(
            Instant::now() < deadline,
            "timed out after {} messages",
            received
        );
        match subscriber.subscribe().unwrap() {
            Some(m) => {
                received += 1;
                let seq = m.header.sequence;
                assert_eq!(seq, received);
                assert_eq!(payload_of(&m), &vec![(seq % 251) as u8; 100]);
            }
            None => std::thread::sleep(Duration::from_micros(100)),
        }
    }
    assert_eq!(subscriber.overruns(), 0);
}
