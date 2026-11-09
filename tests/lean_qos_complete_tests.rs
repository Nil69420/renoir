use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use renoir::topic_rings::{LeanChannel, LeanClock, LeanDurability, LeanQos};

const ROLE: &str = "RENOIR_QOS_ROLE";
const NAME: &str = "RENOIR_QOS_NAME";
const CHILD_TEST: &str = "automatic_liveliness_notices_a_killed_publisher";

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
    format!("qos_{tag}_{}", std::process::id())
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn invalid_combinations_are_rejected_before_anything_is_opened() {
    let n = name("invalid");
    let open = |qos: LeanQos| LeanChannel::open_qos(&n, 16, qos, false, false).is_err();
    assert!(open(LeanQos::best_effort(1)));
    assert!(open(LeanQos::best_effort(8).keep_all()));
    assert!(open(LeanQos::best_effort(8).with_lifespan(ms(5))));
    assert!(open(LeanQos::best_effort(8).with_min_separation(ms(5))));
    assert!(open(
        LeanQos::best_effort(8).with_manual_liveliness(Duration::ZERO)
    ));
    assert!(open(LeanQos::best_effort(8).with_deadline(Duration::ZERO)));
    assert!(!open(LeanQos::reliable(8).keep_all()));
}

#[test]
fn a_reader_must_be_compatible_with_what_the_publisher_offers() {
    let n = name("rxo");
    let offered = LeanQos::best_effort(8)
        .with_deadline(ms(10))
        .with_manual_liveliness(ms(100));
    let _publisher = LeanChannel::open_qos(&n, 16, offered, false, false).unwrap();
    let read = |qos: LeanQos| LeanChannel::open_qos(&n, 16, qos, false, true);
    let same_kind = |q: LeanQos| q.with_manual_liveliness(ms(200));

    assert!(read(same_kind(LeanQos::best_effort(8).with_deadline(ms(20)))).is_ok());
    assert!(read(same_kind(LeanQos::best_effort(8).with_deadline(ms(10)))).is_ok());
    assert!(read(same_kind(LeanQos::best_effort(8).with_deadline(ms(5)))).is_err());
    assert!(read(LeanQos::best_effort(8).with_manual_liveliness(ms(50))).is_err());
    assert!(read(LeanQos::best_effort(8).with_manual_liveliness(ms(100))).is_ok());
    assert!(read(LeanQos::best_effort(8).exclusive(1)).is_err());

    let other = LeanQos::best_effort(8).with_deadline(ms(20));
    assert!(LeanChannel::open_qos(&n, 16, other, false, false).is_err());
}

#[test]
fn a_ring_without_an_offer_accepts_any_reader_request() {
    let n = name("nooffer");
    let _plain = LeanChannel::open(&n, 16, 8, false).unwrap();
    let request = LeanQos::best_effort(8).with_deadline(ms(5));
    assert!(LeanChannel::open_qos(&n, 16, request, false, true).is_ok());
}

#[test]
fn durability_decides_whether_a_late_joiner_sees_retained_samples() {
    let n = name("durable");
    let mut publisher =
        LeanChannel::open_qos(&n, 16, LeanQos::best_effort(8), false, false).unwrap();
    for i in 0..5u8 {
        publisher.publish(&[i; 16]).unwrap();
    }
    let drain = |qos: LeanQos| {
        let mut r = LeanChannel::open_qos(&n, 16, qos, false, true).unwrap();
        let mut out = [0u8; 16];
        let mut count = 0;
        while matches!(
            r.try_read(&mut out),
            Ok(renoir::topic_rings::LeanRead::Message)
        ) {
            count += 1;
        }
        count
    };
    assert_eq!(drain(LeanQos::best_effort(8)), 0);
    assert_eq!(drain(LeanQos::best_effort(8).transient_local()), 5);
    assert_eq!(
        LeanQos::best_effort(8).transient_local().durability,
        LeanDurability::TransientLocal
    );
}

#[test]
fn lifespan_drops_expired_samples_and_counts_them() {
    let n = name("lifespan");
    let qos = LeanQos::best_effort(16)
        .with_clock(LeanClock::Nanoseconds)
        .with_lifespan(ms(30))
        .transient_local();
    let mut publisher = LeanChannel::open_qos(&n, 16, qos, false, false).unwrap();
    publisher.publish_now(&[1u8; 16]).unwrap();
    std::thread::sleep(ms(80));
    publisher.publish_now(&[2u8; 16]).unwrap();
    let mut reader = LeanChannel::open_qos(&n, 16, qos, false, true).unwrap();
    let mut out = [0u8; 16];
    assert!(reader.try_read_stamped_qos(&mut out).unwrap().is_some());
    assert_eq!(out[0], 2);
    assert_eq!(reader.stale_dropped(), 1);
    assert!(reader.try_read_stamped_qos(&mut out).unwrap().is_none());
    assert_eq!(reader.lifespan(), Some(ms(30)));
}

#[test]
fn a_time_based_filter_keeps_one_sample_per_separation() {
    let n = name("filter");
    let qos = LeanQos::best_effort(64)
        .with_clock(LeanClock::Nanoseconds)
        .with_min_separation(ms(5))
        .transient_local();
    let mut publisher = LeanChannel::open_qos(&n, 16, qos, false, false).unwrap();
    for i in 0..21u64 {
        publisher.publish_at(i * 1_000_000, &[i as u8; 16]).unwrap();
    }
    let mut reader = LeanChannel::open_qos(&n, 16, qos, false, true).unwrap();
    let mut out = [0u8; 16];
    let mut delivered = Vec::new();
    while let Some(stamp) = reader.try_read_stamped_qos(&mut out).unwrap() {
        delivered.push(stamp / 1_000_000);
    }
    assert_eq!(delivered, vec![0, 5, 10, 15, 20]);
    assert_eq!(reader.filtered_out(), 16);
}

#[test]
fn manual_liveliness_expires_without_assertions_and_recovers() {
    let n = name("manual");
    let qos = LeanQos::best_effort(8).with_manual_liveliness(ms(80));
    let mut publisher = LeanChannel::open_qos(&n, 16, qos, false, false).unwrap();
    let mut reader = LeanChannel::open_qos(&n, 16, qos, false, true).unwrap();
    assert!(reader.liveliness().lost, "nobody has registered yet");
    publisher.register_publisher().unwrap();
    assert_eq!(reader.liveliness().alive, 1);
    std::thread::sleep(ms(140));
    let status = reader.liveliness();
    assert!(status.lost && status.registered == 1 && status.alive == 0);
    assert_eq!(status.lost_count, 1);
    publisher.assert_liveliness().unwrap();
    let status = reader.liveliness();
    assert!(!status.lost && status.alive == 1);
}

fn registering_child() {
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
    let n = std::env::var(NAME).unwrap();
    let mut channel = LeanChannel::open_qos(&n, 16, LeanQos::best_effort(8), false, false).unwrap();
    channel.register_publisher().unwrap();
    loop {
        std::thread::sleep(ms(50));
    }
}

#[test]
fn automatic_liveliness_notices_a_killed_publisher() {
    if std::env::var(ROLE).is_ok() {
        registering_child();
        return;
    }
    let n = name("auto");
    let mut reader = LeanChannel::open_qos(&n, 16, LeanQos::best_effort(8), false, true).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(ROLE, "child")
        .env(NAME, &n)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while reader.liveliness().alive == 0 {
        assert!(Instant::now() < deadline, "child never registered");
        std::thread::sleep(ms(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let status = reader.liveliness();
    assert!(status.lost && status.registered == 1 && status.alive == 0);
    assert_eq!(status.lost_count, 1);
}

#[test]
fn exclusive_ownership_follows_strength_and_fails_over() {
    let n = name("owner");
    let weak_qos = LeanQos::best_effort(16).exclusive(5);
    let strong_qos = LeanQos::best_effort(16).exclusive(10);
    let mut weak = LeanChannel::open_qos(&n, 16, weak_qos, true, false).unwrap();
    let mut strong = LeanChannel::open_qos(&n, 16, strong_qos, true, false).unwrap();
    weak.register_publisher().unwrap();
    strong.register_publisher().unwrap();

    assert!(
        weak.publish(&[1u8; 16]).is_err(),
        "the weaker publisher must not write"
    );
    strong.publish(&[2u8; 16]).unwrap();
    assert!(strong.is_owner() && !weak.is_owner());

    drop(strong);
    std::thread::sleep(ms(10));
    weak.publish(&[3u8; 16]).unwrap();
    assert!(weak.is_owner());

    let mut reader =
        LeanChannel::open_qos(&n, 16, LeanQos::best_effort(16).exclusive(0), true, true).unwrap();
    assert_eq!(reader.pending(), 0);
    assert!(reader.liveliness().alive >= 1);
}

#[test]
fn exclusive_ownership_with_a_lease_fails_over_from_a_silent_owner() {
    let n = name("ownerlease");
    let qos = |s| {
        LeanQos::best_effort(16)
            .exclusive(s)
            .with_manual_liveliness(ms(80))
    };
    let mut backup = LeanChannel::open_qos(&n, 16, qos(1), true, false).unwrap();
    let mut primary = LeanChannel::open_qos(&n, 16, qos(9), true, false).unwrap();
    backup.assert_liveliness().unwrap();
    primary.assert_liveliness().unwrap();
    std::thread::sleep(ms(10));
    assert!(primary.publish(&[1u8; 16]).is_ok());
    assert!(backup.publish(&[1u8; 16]).is_err());
    std::thread::sleep(ms(60));
    backup.assert_liveliness().unwrap();
    std::thread::sleep(ms(60));
    assert!(
        backup.publish(&[2u8; 16]).is_ok(),
        "the silent primary must lose ownership"
    );
    primary.assert_liveliness().unwrap();
    std::thread::sleep(ms(5));
    assert!(
        primary.publish(&[3u8; 16]).is_ok(),
        "an asserting primary regains it"
    );
}

#[test]
fn ownership_kinds_must_match_between_publisher_and_reader() {
    let shared = name("ownkind_shared");
    let _s = LeanChannel::open_qos(&shared, 16, LeanQos::best_effort(8), true, false).unwrap();
    let wants_exclusive = LeanQos::best_effort(8).exclusive(3);
    let Err(err) = LeanChannel::open_qos(&shared, 16, wants_exclusive, true, true) else {
        panic!("a reader asking for exclusive ownership of a shared ring must be refused");
    };
    assert!(err.to_string().contains("Ownership"), "{err}");

    let exclusive = name("ownkind_excl");
    let _e = LeanChannel::open_qos(&exclusive, 16, wants_exclusive, true, false).unwrap();
    let Err(err) = LeanChannel::open_qos(&exclusive, 16, LeanQos::best_effort(8), true, true)
    else {
        panic!("a shared reader of an exclusive ring must be refused");
    };
    assert!(err.to_string().contains("Ownership"), "{err}");
}
