//! Cross-process pub/sub integration test.
//!
//! The parent creates a topic, publishes a message, and spawns a child
//! process (this same test binary, re-invoked with an env flag) which
//! attaches to the same named shared memory region, receives the parent's
//! message, and replies with its own. This exercises the real cross-process
//! path: ring control blocks and buffers live in the shared region, so both
//! processes observe the same positions and data.
//!
//! `enable_notifications: true` is used on purpose — that is the exact
//! configuration mujoco_drones' transport layer uses. Topics must get
//! FileBacked (cross-process-capable) regions regardless of the notification
//! setting, so this configuration must round-trip across processes.

use std::{process::Command, time::Duration};

use renoir::{
    topic::{MessagePayload, TopicConfig, TopicPattern},
    topic_manager_modules::TopicManager,
};

const CHILD_ENV: &str = "RENOIR_XPROC_CHILD";
const TOPIC_ENV: &str = "RENOIR_XPROC_TOPIC";
const PARENT_PAYLOAD: &str = "hello-from-parent";
const CHILD_PAYLOAD: &str = "hello-from-child";
const CRASH_CHILD_ENV: &str = "RENOIR_XPROC_CRASH_CHILD";
const CRASH_TOPIC_ENV: &str = "RENOIR_XPROC_CRASH_TOPIC";
const OWNER_PAYLOAD: &str = "owner-session-message";
const ONESHOT_CHILD_ENV: &str = "RENOIR_XPROC_ONESHOT_CHILD";
const ONESHOT_TOPIC_ENV: &str = "RENOIR_XPROC_ONESHOT_TOPIC";
const ONESHOT_PAYLOAD: &str = "one-shot-owner-message";

#[test]
fn test_cross_process_pub_sub() {
    if std::env::var(CHILD_ENV).is_ok() {
        run_child();
        return;
    }
    run_parent();
}

fn topic_config(name: &str) -> TopicConfig {
    TopicConfig {
        name: name.to_string(),
        pattern: TopicPattern::MPMC,
        // The exact setting mujoco_drones uses (transport_renoir.c sets
        // enable_notifications: true). Since Step 22, topics are always
        // FileBacked regardless, so this must still round-trip.
        enable_notifications: true,
        ..Default::default()
    }
}

fn run_parent() {
    let topic_name = format!("xproc_{}", std::process::id());
    let manager = TopicManager::new().unwrap();
    manager.create_topic(topic_config(&topic_name)).unwrap();

    let publisher = manager.create_publisher(&topic_name).unwrap();
    let subscriber = manager.create_subscriber(&topic_name).unwrap();
    publisher
        .publish(PARENT_PAYLOAD.as_bytes().to_vec())
        .unwrap();

    let exe = std::env::current_exe().unwrap();
    let status = Command::new(exe)
        .args(["--exact", "test_cross_process_pub_sub", "--nocapture"])
        .env(CHILD_ENV, "1")
        .env(TOPIC_ENV, &topic_name)
        .status()
        .expect("failed to spawn child process");

    assert!(status.success(), "child process failed: {}", status);

    let mut received = None;
    for _ in 0..500 {
        if let Some(message) = subscriber.subscribe().unwrap() {
            received = Some(message);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let message = received.expect("parent never received child's reply");
    match message.payload {
        MessagePayload::Inline(data) => {
            assert_eq!(data, CHILD_PAYLOAD.as_bytes());
        }
        other => panic!("expected inline payload, got {:?}", other),
    }

    drop(publisher);
    drop(subscriber);
    manager.remove_topic(&topic_name).unwrap();
}

fn run_child() {
    let topic_name = std::env::var(TOPIC_ENV).expect("child missing topic env var");
    let manager = TopicManager::new().unwrap();
    manager.create_topic(topic_config(&topic_name)).unwrap();

    let subscriber = manager.create_subscriber(&topic_name).unwrap();

    let mut received = None;
    for _ in 0..500 {
        if let Some(message) = subscriber.subscribe().unwrap() {
            received = Some(message);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let message = received.expect("child never received parent's message");
    match message.payload {
        MessagePayload::Inline(data) => {
            assert_eq!(data, PARENT_PAYLOAD.as_bytes());
        }
        other => panic!("expected inline payload, got {:?}", other),
    }

    let publisher = manager.create_publisher(&topic_name).unwrap();
    publisher
        .publish(CHILD_PAYLOAD.as_bytes().to_vec())
        .unwrap();
}

/// Parent side: spawn a child that creates the topic region (the session
/// owner), publishes, and signals readiness. We attach as a participant,
/// verify the owner's message, SIGKILL the child-owner, then re-attach.
/// The region must be wiped — the owner died with the DIRTY marker, so no
/// leftover data from the dead session survives.
fn run_crash_parent() {
    let topic_name = format!("crash_{}", std::process::id());
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new(exe)
        .args([
            "--exact",
            "test_crashed_child_region_is_reinitialized",
            "--nocapture",
        ])
        .env(CRASH_CHILD_ENV, "1")
        .env(CRASH_TOPIC_ENV, &topic_name)
        .spawn()
        .expect("failed to spawn crash child");

    let ready_path = crash_ready_path(child.id());

    // Wait until the child-owner has created the region, published its
    // message, and signalled readiness — only then is it safe for us to
    // attach as a participant (had we created the topic first, we would
    // have become the session owner and the test would not be testing a
    // crash of the owner).
    let mut ready = false;
    for _ in 0..500 {
        if ready_path.exists() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        ready,
        "child owner never signalled readiness before timing out"
    );

    // Attach as a participant: the child already owns the session, so our
    // exclusive-lock attempt must fail and we must never touch the marker.
    let manager = TopicManager::new().unwrap();
    manager.create_topic(topic_config(&topic_name)).unwrap();
    let subscriber = manager.create_subscriber(&topic_name).unwrap();

    let mut received = None;
    for _ in 0..500 {
        if let Some(message) = subscriber.subscribe().unwrap() {
            received = Some(message);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let message = received.expect("parent never received the owner's message");
    match message.payload {
        MessagePayload::Inline(data) => {
            assert_eq!(data, OWNER_PAYLOAD.as_bytes());
        }
        other => panic!("expected inline payload, got {:?}", other),
    }

    // Simulate a crash of the owner: SIGKILL before it can run any cleanup.
    // The kernel releases its flock when the fds close, leaving the DIRTY
    // marker behind.
    child.kill().expect("failed to kill crash child");
    let status = child.wait().expect("failed to wait for crash child");
    assert!(!status.success());

    // Release our participant handles. We were never the owner, so we must
    // not have written the CLEAN marker.
    drop(subscriber);
    drop(manager);
    let _ = std::fs::remove_file(&ready_path);

    // Re-attach. The owner died with the DIRTY marker, so this must get a
    // fresh (zeroed) region, not the dead session's leftover data.
    let fresh_manager = TopicManager::new().unwrap();
    fresh_manager
        .create_topic(topic_config(&topic_name))
        .unwrap();
    let fresh_subscriber = fresh_manager.create_subscriber(&topic_name).unwrap();
    assert!(
        fresh_subscriber.subscribe().unwrap().is_none(),
        "fresh region must not contain the dead owner's leftover message"
    );

    // A clean round trip must work on the fresh region
    let fresh_publisher = fresh_manager.create_publisher(&topic_name).unwrap();
    fresh_publisher
        .publish(b"post-crash message".to_vec())
        .unwrap();
    let message = fresh_subscriber
        .subscribe()
        .unwrap()
        .expect("fresh message available");
    match message.payload {
        MessagePayload::Inline(data) => {
            assert_eq!(data, b"post-crash message".to_vec());
        }
        other => panic!("expected inline payload, got {:?}", other),
    }

    drop(fresh_publisher);
    drop(fresh_subscriber);
    fresh_manager.remove_topic(&topic_name).unwrap();
}

/// Child side of the crash test: create the topic (becoming the session
/// owner), publish a message, signal readiness, then sleep forever without
/// any cleanup (the parent SIGKILLs this process). The DIRTY marker must
/// survive our death.
fn run_crash_child() {
    let topic_name = std::env::var(CRASH_TOPIC_ENV).expect("child missing crash topic env var");
    let manager = TopicManager::new().unwrap();
    manager.create_topic(topic_config(&topic_name)).unwrap();

    let publisher = manager.create_publisher(&topic_name).unwrap();
    publisher
        .publish(OWNER_PAYLOAD.as_bytes().to_vec())
        .unwrap();

    // Signal readiness, then sleep forever; the parent SIGKILLs us. No
    // remove_topic, no drops: the DIRTY marker must survive our death.
    std::fs::write(crash_ready_path(std::process::id()), b"1").unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

#[test]
fn test_crashed_child_region_is_reinitialized() {
    if std::env::var(CRASH_CHILD_ENV).is_ok() {
        run_crash_child();
        return;
    }
    run_crash_parent();
}

/// Path of the readiness sentinel file for a child-owner process. The child
/// writes it after publishing, so the parent can attach as a participant
/// without racing to become the owner itself.
fn crash_ready_path(child_pid: u32) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("renoir_crash_ready_{}", child_pid))
}

/// Parent side: spawn a child that creates the topic (becoming the session
/// owner), publishes one message, and exits normally — its Drop marks the
/// region CLEAN. A later opener must preserve that message, not wipe it:
/// this is the one-shot-publisher scenario that flock-based liveness used
/// to destroy (analysis §11.5).
fn run_oneshot_parent() {
    let topic_name = format!("oneshot_{}", std::process::id());
    let exe = std::env::current_exe().unwrap();
    let status = Command::new(exe)
        .args([
            "--exact",
            "test_oneshot_owner_message_survives_clean_exit",
            "--nocapture",
        ])
        .env(ONESHOT_CHILD_ENV, "1")
        .env(ONESHOT_TOPIC_ENV, &topic_name)
        .status()
        .expect("failed to spawn one-shot child");

    assert!(status.success(), "one-shot child failed: {}", status);

    // The previous session (the child) closed cleanly, so the opener must
    // preserve its message rather than wiping it.
    let manager = TopicManager::new().unwrap();
    manager.create_topic(topic_config(&topic_name)).unwrap();
    let subscriber = manager.create_subscriber(&topic_name).unwrap();

    let message = subscriber
        .subscribe()
        .unwrap()
        .expect("one-shot publisher's message must survive its clean exit");
    match message.payload {
        MessagePayload::Inline(data) => {
            assert_eq!(data, ONESHOT_PAYLOAD.as_bytes());
        }
        other => panic!("expected inline payload, got {:?}", other),
    }

    drop(subscriber);
    manager.remove_topic(&topic_name).unwrap();
}

/// Child side of the one-shot test: create the topic (session owner),
/// publish a single message, then return — the normal exit path writes the
/// CLEAN marker, marking the published message as legitimate.
fn run_oneshot_child() {
    let topic_name =
        std::env::var(ONESHOT_TOPIC_ENV).expect("child missing one-shot topic env var");
    let manager = TopicManager::new().unwrap();
    manager.create_topic(topic_config(&topic_name)).unwrap();

    let publisher = manager.create_publisher(&topic_name).unwrap();
    publisher
        .publish(ONESHOT_PAYLOAD.as_bytes().to_vec())
        .unwrap();
}

#[test]
fn test_oneshot_owner_message_survives_clean_exit() {
    if std::env::var(ONESHOT_CHILD_ENV).is_ok() {
        run_oneshot_child();
        return;
    }
    run_oneshot_parent();
}
