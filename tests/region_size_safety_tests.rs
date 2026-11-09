use std::os::unix::fs::MetadataExt;

use renoir::topic_rings::{LeanChannel, LeanMode};

fn dir() -> &'static std::path::Path {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("RENOIR_SHM_DIR", dir.path());
        dir
    })
    .path()
}

#[test]
fn a_rejected_attach_with_smaller_geometry_never_shrinks_a_live_ring() {
    let d = dir();
    let name = format!("size_small_{}", std::process::id());
    let mut big =
        LeanChannel::open_with_mode(&name, 16, 1024, false, LeanMode::SingleProducer).unwrap();
    let path = d.join(format!("renoir_{name}"));
    let before = std::fs::metadata(&path).unwrap().len();
    for tries in 0..3 {
        let small = LeanChannel::open_with_mode(&name, 16, 8, false, LeanMode::SingleProducer);
        assert!(
            small.is_err(),
            "attempt {tries}: mismatched geometry must be rejected"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            before,
            "attempt {tries}"
        );
    }
    for n in 0..4000u64 {
        big.publish(&[n as u8; 16]).unwrap();
    }
}

#[test]
fn a_rejected_attach_with_larger_geometry_neither_grows_nor_crashes() {
    let d = dir();
    let name = format!("size_large_{}", std::process::id());
    let mut small =
        LeanChannel::open_with_mode(&name, 16, 8, false, LeanMode::SingleProducer).unwrap();
    let path = d.join(format!("renoir_{name}"));
    let before = std::fs::metadata(&path).unwrap().len();
    let started = std::time::Instant::now();
    let large = LeanChannel::open_with_mode(&name, 16, 1024, false, LeanMode::SingleProducer);
    assert!(large.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
    for n in 0..100u64 {
        small.publish(&[n as u8; 16]).unwrap();
    }
}

#[test]
fn creating_a_ring_reserves_its_pages_up_front() {
    let d = dir();
    let name = format!("size_reserved_{}", std::process::id());
    let _ring =
        LeanChannel::open_with_mode(&name, 16, 4096, false, LeanMode::SingleProducer).unwrap();
    let meta = std::fs::metadata(d.join(format!("renoir_{name}"))).unwrap();
    assert!(
        meta.blocks() * 512 >= meta.len(),
        "file is sparse: {} bytes allocated of {}",
        meta.blocks() * 512,
        meta.len()
    );
}

#[test]
fn concurrent_first_opens_of_the_same_ring_all_succeed() {
    let d = dir();
    for round in 0..20 {
        let name = format!("size_race_{}_{round}", std::process::id());
        let barrier = std::sync::Barrier::new(6);
        std::thread::scope(|scope| {
            for _ in 0..6 {
                let (barrier, name) = (&barrier, &name);
                scope.spawn(move || {
                    barrier.wait();
                    let mut ch =
                        LeanChannel::open_with_mode(name, 16, 256, false, LeanMode::SingleProducer)
                            .expect("a concurrent first open must not fail");
                    let _ = ch.publish(&[1u8; 16]);
                });
            }
        });
        assert!(d.join(format!("renoir_{name}")).exists());
    }
}

#[test]
fn opening_a_large_ring_prefaults_it_so_the_first_publishes_do_not_page_fault() {
    let _ = dir();
    let name = format!("size_prefault_{}", std::process::id());
    let minflt = || -> u64 {
        let stat = std::fs::read_to_string("/proc/thread-self/stat").unwrap();
        let rest = &stat[stat.rfind(')').unwrap() + 2..];
        rest.split_whitespace().nth(7).unwrap().parse().unwrap()
    };
    let mut ring =
        LeanChannel::open_with_mode(&name, 56, 16384, false, LeanMode::SingleProducer).unwrap();
    let before = minflt();
    for n in 0..16384u64 {
        ring.publish(&[n as u8; 56]).unwrap();
    }
    let faults = minflt() - before;
    assert!(
        faults < 16,
        "publishing across a 1 MiB ring took {faults} minor page faults; the mapping was not prefaulted"
    );
}
