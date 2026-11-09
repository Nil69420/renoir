use std::ptr::NonNull;
use std::sync::Arc;

use renoir::topic_rings::{LeanClock, LeanMode, LeanRing, STAMP_BYTES};

fn ring(mode: LeanMode, data: usize, clock: LeanClock) -> (Arc<LeanRing>, Vec<u64>) {
    let msg = data + STAMP_BYTES;
    let ring_bytes = renoir::topic_rings::lean_slot_bytes(msg).unwrap() * 64;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let mut mem = vec![0u64; total / 8 + 1];
    let ptr = NonNull::new(mem.as_mut_ptr().cast::<u8>()).unwrap();
    let r = unsafe {
        LeanRing::from_memory_stamped(ptr, mem.len() * 8, ring_bytes, data, mode, clock).unwrap()
    };
    (Arc::new(r), mem)
}

#[test]
fn stamps_round_trip_in_every_mode() {
    for mode in [
        LeanMode::SingleProducer,
        LeanMode::MultiProducer,
        LeanMode::SingleProducerGated,
        LeanMode::MultiProducerGated,
    ] {
        for data in [8usize, 48, 56, 100, 496] {
            let (ring, _mem) = ring(mode, data, LeanClock::Nanoseconds);
            assert_eq!(ring.data_size(), data);
            let mut p = ring.publisher();
            let mut cur = ring.cursor_oldest();
            let mut gated = mode
                .is_gated()
                .then(|| ring.register_consumer(true).unwrap());
            let mut out = vec![0u8; data];
            for i in 0..10u64 {
                let payload = vec![i as u8 + 1; data];
                p.publish_at(1000 + i, &payload).unwrap();
                let got = match gated.as_mut() {
                    Some(g) => ring.try_read_stamped_gated(g, &mut out).unwrap(),
                    None => ring.try_read_stamped(&mut cur, &mut out).unwrap(),
                };
                assert_eq!(got, Some(1000 + i), "{mode:?} {data}");
                assert_eq!(out, payload);
            }
            let empty = match gated.as_mut() {
                Some(g) => ring.try_read_stamped_gated(g, &mut out).unwrap(),
                None => ring.try_read_stamped(&mut cur, &mut out).unwrap(),
            };
            assert_eq!(empty, None);
        }
    }
}

#[test]
fn publish_now_is_monotonic_and_ticks_convert() {
    for clock in [LeanClock::Nanoseconds, LeanClock::Ticks] {
        let (ring, _mem) = ring(LeanMode::SingleProducer, 16, clock);
        let mut p = ring.publisher();
        let mut cur = ring.cursor_oldest();
        let mut out = [0u8; 16];
        for _ in 0..50 {
            p.publish_now(&[1u8; 16]).unwrap();
        }
        let mut last = 0;
        for _ in 0..50 {
            let s = ring.try_read_stamped(&mut cur, &mut out).unwrap().unwrap();
            assert!(s >= last);
            last = s;
        }
        if clock == LeanClock::Ticks {
            assert!(ring.tick_hz() > 1_000_000);
            assert!(ring.ticks_to_ns(ring.tick_hz()) == 1_000_000_000);
        }
    }
}

#[test]
fn unstamped_and_oversized_use_is_rejected() {
    let (ring, _mem) = ring(LeanMode::SingleProducer, 16, LeanClock::Nanoseconds);
    let mut p = ring.publisher();
    assert!(p.publish_at(1, &[0u8; 15]).is_err());
    assert!(p.publish(&[0u8; 16]).is_err());
    let mut small = [0u8; 8];
    let mut cur = ring.cursor_oldest();
    p.publish_at(1, &[0u8; 16]).unwrap();
    assert!(ring.try_read_stamped(&mut cur, &mut small).is_err());
    let mut mem = vec![0u64; 4096];
    let ptr = NonNull::new(mem.as_mut_ptr().cast::<u8>()).unwrap();
    assert!(unsafe {
        LeanRing::from_memory_stamped(
            ptr,
            4096 * 8,
            4096,
            500,
            LeanMode::SingleProducer,
            LeanClock::Nanoseconds,
        )
    }
    .is_err());
}

#[test]
fn tick_conversion_is_monotonic_and_exact_at_real_counter_rates() {
    let (ring, _mem) = ring(LeanMode::SingleProducer, 16, LeanClock::Ticks);
    let hz = ring.tick_hz();
    assert!(hz > 0);
    assert_eq!(ring.ticks_to_ns(hz), 1_000_000_000);
    assert_eq!(ring.ticks_to_ns(0), 0);
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut previous = (0u64, 0u64);
    let mut samples = Vec::with_capacity(100_000);
    for _ in 0..100_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        samples.push(x >> 2);
    }
    samples.sort_unstable();
    for t in samples {
        let ns = ring.ticks_to_ns(t);
        assert!(ns >= previous.1, "conversion not monotonic at {t}");
        previous = (t, ns);
    }
}

#[test]
fn concurrent_reader_never_sees_a_stamp_from_another_message() {
    let rounds: u64 = if cfg!(miri) { 200 } else { 300_000 };
    for data in [8usize, 56, 120] {
        let (ring, _mem) = ring(LeanMode::SingleProducer, data, LeanClock::Nanoseconds);
        let writer_ring = ring.clone();
        let writer = std::thread::spawn(move || {
            let mut p = writer_ring.publisher();
            let mut payload = vec![0u8; data];
            for i in 1..=rounds {
                payload[..8].copy_from_slice(&i.to_ne_bytes());
                p.publish_at(i, &payload).unwrap();
            }
        });
        let mut cursor = ring.cursor_oldest();
        let mut out = vec![0u8; data];
        let mut seen = 0u64;
        let mut last = 0u64;
        while last < rounds {
            if let Some(stamp) = ring.try_read_stamped(&mut cursor, &mut out).unwrap() {
                assert_eq!(stamp, u64::from_ne_bytes(out[..8].try_into().unwrap()));
                assert!(stamp > last);
                last = stamp;
                seen += 1;
            } else {
                std::hint::spin_loop();
            }
        }
        writer.join().unwrap();
        assert!(seen > 0);
    }
}

#[test]
fn lag_is_reported_on_the_read_that_observes_it() {
    use renoir::topic_rings::{LeanChannel, LeanRead};
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("RENOIR_SHM_DIR", dir.path());
    let name = format!("lag_{}", std::process::id());
    let mut writer = LeanChannel::open(&name, 16, 16, true).unwrap();
    let mut reader = LeanChannel::open(&name, 16, 16, true).unwrap();
    let mut out = [0u8; 16];
    assert_eq!(
        reader.try_read_lagged(&mut out).unwrap(),
        (LeanRead::Empty, 0)
    );
    let published = 200u64;
    for i in 1..=published {
        let mut payload = [0u8; 16];
        payload[..8].copy_from_slice(&i.to_ne_bytes());
        writer.publish(&payload).unwrap();
    }
    let mut read = 0u64;
    let mut missed = 0u64;
    loop {
        match reader.try_read_lagged(&mut out).unwrap() {
            (LeanRead::Message, m) => {
                read += 1;
                missed += m;
            }
            (LeanRead::Empty, m) => {
                missed += m;
                break;
            }
        }
    }
    assert!(missed > 0, "a 16-slot ring that took 200 messages must lag");
    assert_eq!(read + missed, published);
    assert_eq!(missed, reader.lost());
}
