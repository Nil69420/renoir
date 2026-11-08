# Benchmarks

All numbers are one-way latency (half the round trip) at the median, measured between two threads pinned to different physical cores of an Intel i7-11800H (performance governor, constant TSC, Linux 6.8, release build) with a spinning consumer. Compare rows within the same table only; absolute values drift by about 10 ns between sessions. Reproduce with `cargo bench --bench latency_pingpong -- 2 4` and `cargo bench --manifest-path benches/comparison/Cargo.toml --bench vs_iceoryx2 -- 2 4`.

| payload | Renoir lean ring | Renoir broadcast, copy out | Renoir broadcast, in-place view | iceoryx2 0.10 |
|---|---|---|---|---|
| 56 B | 66 ns | 277 ns | 181 ns | 421-436 ns |
| 1 KiB | - | 374 ns | 156 ns | 422-495 ns |
| 64 KiB | - | 6.7 us | 155 ns | 385 ns - 3.9 us |
| 921,600 B (camera frame) | - | 96.6 us | 178 ns | 405 ns - 43.5 us |

The raw cache-line transfer floor on this machine is 66 ns, so the lean ring is within measurement noise of what the hardware allows. iceoryx2 ranges show its zero-copy mode (pointer handover only) and its copy mode (producer writes the bytes, consumer reads them in place).

Request-response round trip (spinning server): 243 ns for 8 bytes, 331 ns for 64 bytes, 395 ns for 256 bytes.

## What the numbers do not say

- Zero-copy and in-place rows measure handing over a pointer; the payload is neither produced nor consumed. A real application still pays for producing the bytes.
- The in-place view is validated after use and reports when the producer overwrote the slot; iceoryx2 samples stay owned by the consumer until dropped. Slow consumers are safer with iceoryx2; consumers that keep up are cheaper with Renoir.
- Latency tails (p99.9 and above) are dominated by operating system jitter, not by the rings. Bounding them needs `isolcpus`, `nohz_full` and interrupt affinity.
- Measurements are x86 only. The Jetson Orin Nano (Cortex-A78AE) has a slower generic timer and different cache-line transfer costs and must be measured on the device.
- QEMU does not reproduce hardware memory reordering. The ring protocols are checked against the AArch64 memory model with herd7 (`tests/litmus`) and stress-tested natively on arm64 CI runners.

## Method notes

Payload buffers and per-thread state are cache-line isolated and the ring memory is page aligned, because a benchmark that lets two threads share a cache line through the allocator can move a result by 10 to 20 ns. Before every run we check that nothing else is spinning on the pinned cores; one stray process once polluted a whole session of our own measurements.

## Running on the Jetson Orin Nano

The Orin's generic timer ticks at about 31 MHz, roughly 32 ns per tick, so a single 140 ns round trip cannot be timed directly. On non-x86 targets the ping-pong bench therefore times bursts: it measures 512 consecutive round trips with one pair of timestamps and reports the per-round-trip mean, so the quantisation error shrinks by a factor of 512. Percentiles in that mode are percentiles of burst means, not of individual round trips. Override the burst size with `BENCH_BURST=<n>` (use 1 on x86).

```
sudo nvpmodel -m 0 && sudo jetson_clocks
BENCH_BURST=512 cargo bench --bench latency_pingpong -- 2 4
BENCH_BURST=512 cargo bench --manifest-path benches/comparison/Cargo.toml --bench vs_iceoryx2 -- 2 4
```

Pick two different physical cores that your real-time threads do not use, and record the CPU model, kernel, `nvpmodel` mode and governor next to the numbers. Latency below the burst resolution cannot be claimed from this mode, and a hardware cycle counter (PMU) based timer has not been implemented.

## Frame rings against iceoryx2 zero-copy

One-way p50 latency, owned zero-copy on both sides, cores 2 and 4 of an i7-11800H, single run (`cd benches/comparison && cargo bench --bench frames_vs_iceoryx2`):

| Payload | renoir frame ring | iceoryx2 0.10 zero-copy |
|---|---|---|
| 56 B | 95 ns | 400 ns |
| 1 KiB | 96 ns | 414 ns |
| 64 KiB | 113 ns | 383 ns |
| 921,600 B | 95 ns | 398 ns |

With each side touching the first 64 B: 124 to 137 ns against 429 to 447 ns. These rows measure ownership hand-over, not data movement; a frame that is actually written and read costs what the memory system charges for it in both libraries. iceoryx2 was run in its default configuration and offers dynamic subscribers and history that Renoir's frame ring does not. The numbers are from one run on one machine.
