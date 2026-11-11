use std::time::{Duration, Instant};

use renoir::topic_rings::{FrameChannel, LeanClock, LeanQos};

fn shm_dir() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    });
}

fn name(tag: &str) -> String {
    shm_dir();
    format!("fqos_{tag}_{}", std::process::id())
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn publish(channel: &mut FrameChannel, byte: u8) -> renoir::error::Result<()> {
    let mut loan = channel.loan(64)?;
    loan.bytes_mut().fill(byte);
    loan.commit()
}

#[test]
fn frame_rings_refuse_policies_that_do_not_apply() {
    let n = name("invalid");
    let open = |qos: LeanQos| FrameChannel::open_qos(&n, 64, qos, false, false).is_err();
    assert!(open(LeanQos::best_effort(8)));
    assert!(open(
        LeanQos::reliable(8).with_clock(LeanClock::Nanoseconds)
    ));
    assert!(open(
        LeanQos::reliable(8)
            .with_clock(LeanClock::Nanoseconds)
            .with_min_separation(ms(5))
    ));
    assert!(open(LeanQos::reliable(1)));
    assert!(!open(LeanQos::reliable(8)));
}

#[test]
fn frame_readers_are_checked_against_the_offer() {
    let n = name("rxo");
    let offered = LeanQos::reliable(8)
        .with_deadline(ms(10))
        .with_manual_liveliness(ms(100));
    let _p = FrameChannel::open_qos(&n, 64, offered, false, false).unwrap();
    let read = |qos: LeanQos| FrameChannel::open_qos(&n, 64, qos, false, true).is_ok();
    let manual = |q: LeanQos| q.with_manual_liveliness(ms(200));
    assert!(read(manual(LeanQos::reliable(8).with_deadline(ms(20)))));
    assert!(!read(manual(LeanQos::reliable(8).with_deadline(ms(5)))));
    assert!(!read(LeanQos::reliable(8).with_manual_liveliness(ms(50))));
    assert!(!read(manual(LeanQos::reliable(8).exclusive(1))));
}

#[test]
fn a_frame_lifespan_drops_expired_frames_for_a_consumer() {
    let n = name("life");
    let qos = LeanQos::reliable(8).with_lifespan(ms(40)).transient_local();
    let mut producer = FrameChannel::open_qos(&n, 64, qos, false, false).unwrap();
    let mut consumer = FrameChannel::open_qos(&n, 64, qos, false, true).unwrap();
    publish(&mut producer, 1).unwrap();
    std::thread::sleep(ms(120));
    publish(&mut producer, 2).unwrap();
    let view = consumer.view_qos().unwrap().expect("the fresh frame");
    assert_eq!(view.bytes()[0], 2);
    view.release();
    assert_eq!(consumer.stale_dropped(), 1);
    assert!(consumer.view_qos().unwrap().is_none());
    assert_eq!(consumer.lifespan(), Some(ms(40)));
}

#[test]
fn frame_publisher_liveliness_and_waiting() {
    let n = name("live");
    let qos = LeanQos::reliable(8).with_manual_liveliness(ms(80));
    let mut producer = FrameChannel::open_qos(&n, 64, qos, false, false).unwrap();
    let mut consumer = FrameChannel::open_qos(&n, 64, qos, false, true).unwrap();
    assert!(consumer.liveliness().lost);
    producer.register_publisher().unwrap();
    assert_eq!(consumer.liveliness().alive, 1);
    let started = Instant::now();
    let status = consumer
        .wait_liveliness_change(Some(Duration::from_secs(5)))
        .expect("the lease expires");
    assert!(status.lost && status.lost_count == 1);
    assert!(started.elapsed() < Duration::from_secs(2));
    publish(&mut producer, 3).unwrap();
    assert!(
        !consumer.liveliness().lost,
        "a frame commit path counts as an assertion"
    );
}

#[test]
fn exclusive_frame_ownership_refuses_the_weaker_publisher_and_fails_over() {
    let n = name("owner");
    let weak_q = LeanQos::reliable(8).exclusive(2);
    let strong_q = LeanQos::reliable(8).exclusive(7);
    let mut weak = FrameChannel::open_qos(&n, 64, weak_q, true, false).unwrap();
    let mut strong = FrameChannel::open_qos(&n, 64, strong_q, true, false).unwrap();
    weak.register_publisher().unwrap();
    strong.register_publisher().unwrap();
    assert!(
        weak.loan(64).is_err(),
        "the weaker publisher must not get a loan"
    );
    publish(&mut strong, 5).unwrap();
    assert!(strong.is_owner() && !weak.is_owner());
    drop(strong);
    std::thread::sleep(ms(10));
    publish(&mut weak, 6).unwrap();
    assert!(weak.is_owner());
}

#[test]
fn a_frame_consumer_counts_missed_deadlines() {
    let n = name("deadline");
    let qos = LeanQos::reliable(8).with_deadline(ms(15));
    let _p = FrameChannel::open_qos(&n, 64, qos, false, false).unwrap();
    let mut consumer = FrameChannel::open_qos(&n, 64, qos, false, true).unwrap();
    assert_eq!(
        consumer.wait_deadline(10),
        renoir::topic_rings::DeadlineWait::Missed
    );
    assert_eq!(
        consumer.wait_deadline(10),
        renoir::topic_rings::DeadlineWait::Missed
    );
    assert_eq!(consumer.deadline_misses(), 2);
}
