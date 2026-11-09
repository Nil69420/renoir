mod support;

use support::cases::*;
use support::timing::*;

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let (a, b) = (
        args.first().copied().unwrap_or(2),
        args.get(1).copied().unwrap_or(4),
    );
    let tpn = ticks_per_ns();
    println!("cores {a} <-> {b}, {tpn:.3} ticks/ns, {WARMUP} warmup, {SAMPLES} samples");
    raw_floor(a, b, tpn);
    lean_pingpong(a, b, tpn);
    real_lean_pingpong(a, b, 56, tpn);
    real_lean_pingpong(a, b, 48, tpn);
    real_lean_pingpong(a, b, 16, tpn);
    real_lean_pingpong(a, b, 120, tpn);
    ffi_pingpong(a, b, 56, tpn);
    ffi_pingpong(a, b, 16, tpn);
    ring_pingpong(a, b, 56, tpn);
    ring_pingpong(a, b, 8, tpn);
}
