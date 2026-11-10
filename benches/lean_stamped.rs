use std::hint::black_box;
use std::time::Instant;

use renoir::topic_rings::{lean_slot_bytes, LeanClock, LeanMode, LeanRing, STAMP_BYTES};

const DATA: usize = 48;
const ROUNDS: usize = 3_000_000;

fn make(clock: LeanClock) -> (std::sync::Arc<LeanRing>, Vec<u64>) {
    let msg = if clock.is_stamped() {
        DATA + STAMP_BYTES
    } else {
        DATA
    };
    let ring_bytes = lean_slot_bytes(msg).unwrap() * 1024;
    let total = LeanRing::required_memory_size(ring_bytes, msg);
    let mut mem = vec![0u64; total / 8 + 1024];
    let base = (mem.as_mut_ptr() as usize + 4095) & !4095;
    let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
    let ring = unsafe {
        if clock.is_stamped() {
            LeanRing::from_memory_stamped(
                ptr,
                total,
                ring_bytes,
                DATA,
                LeanMode::SingleProducer,
                clock,
            )
        } else {
            LeanRing::from_memory_mode(ptr, total, ring_bytes, DATA, LeanMode::SingleProducer)
        }
        .unwrap()
    };
    (std::sync::Arc::new(ring), mem)
}

fn best(mut f: impl FnMut()) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..7 {
        let t0 = Instant::now();
        for _ in 0..ROUNDS {
            f();
        }
        b = b.min(t0.elapsed().as_nanos() as f64 / ROUNDS as f64);
    }
    b
}

fn main() {
    let data = [7u8; DATA];
    let mut out = [0u8; DATA];
    {
        let (ring, _m) = make(LeanClock::None);
        let mut p = ring.publisher();
        let mut c = ring.cursor_latest();
        let t = best(|| {
            p.publish(&data).unwrap();
            let _ = black_box(ring.try_read(&mut c, &mut out));
        });
        println!("plain publish+read             {t:6.2} ns");
    }
    for (clock, label) in [(LeanClock::Nanoseconds, "ns"), (LeanClock::Ticks, "ticks")] {
        let (ring, _m) = make(clock);
        let mut p = ring.publisher();
        let mut c = ring.cursor_latest();
        let t = best(|| {
            p.publish_at(black_box(12345), &data).unwrap();
            let _ = black_box(ring.try_read_stamped(&mut c, &mut out));
        });
        println!("stamped publish_at ({label:<5})     {t:6.2} ns");
        let t = best(|| {
            p.publish_now(&data).unwrap();
            let _ = black_box(ring.try_read_stamped(&mut c, &mut out));
        });
        println!("stamped publish_now ({label:<5})    {t:6.2} ns");
    }
}
