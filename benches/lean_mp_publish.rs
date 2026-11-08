use std::time::Instant;

use renoir::topic_rings::{lean_slot_bytes, LeanMode, LeanRead, LeanRing};

const MSG: usize = 56;
const ROUNDS: usize = 3_000_000;

fn run(mode: LeanMode, label: &str) {
    let ring_bytes = lean_slot_bytes(MSG).unwrap() * 1024;
    let total = LeanRing::required_memory_size(ring_bytes, MSG);
    let mut mem = vec![0u64; total / 8 + 1024];
    let base = (mem.as_mut_ptr() as usize + 4095) & !4095;
    let ptr = std::ptr::NonNull::new(base as *mut u8).unwrap();
    let ring = std::sync::Arc::new(unsafe {
        LeanRing::from_memory_mode(ptr, total, ring_bytes, MSG, mode).unwrap()
    });
    let mut publisher = ring.publisher();
    let data = [7u8; MSG];
    let mut out = [0u8; MSG];
    let mut best = f64::MAX;
    for _ in 0..7 {
        let mut gated = mode
            .is_gated()
            .then(|| ring.register_consumer(true).unwrap());
        let mut cursor = ring.cursor_latest();
        let t0 = Instant::now();
        for _ in 0..ROUNDS {
            publisher.publish(&data).unwrap();
            match gated.as_mut() {
                Some(g) => {
                    let _ = ring.try_read_gated(g, &mut out);
                }
                None => {
                    if let Ok(LeanRead::Message) = ring.try_read(&mut cursor, &mut out) {
                        std::hint::black_box(&out);
                    }
                }
            }
        }
        best = best.min(t0.elapsed().as_nanos() as f64 / ROUNDS as f64);
        if let Some(mut g) = gated {
            ring.unregister_consumer(&mut g);
        }
    }
    println!("{label:<28} publish+read best-of-7: {best:6.2} ns");
}

fn main() {
    run(LeanMode::SingleProducer, "single producer");
    run(LeanMode::MultiProducer, "multi producer");
    run(LeanMode::MultiProducerGated, "multi producer gated");
}
