# Lean ring: sub-100 ns small messages

The lean ring is a single-writer, many-reader broadcast ring for small fixed-size
messages such as IMU samples and poses. One message occupies one 64-byte cache line
(8-byte sequence word plus up to 56 bytes of payload). Larger messages use up to
8 lines (504 bytes). The slot count is rounded down to a power of two.

## C usage

```c
RenoirLeanHandle pub, sub;
renoir_lean_open("imu", sizeof(struct imu), 1024, 0, &pub);
renoir_lean_open("imu", sizeof(struct imu), 1024, 0, &sub);

renoir_lean_publish(pub, (const uint8_t *)&sample, sizeof sample);

struct imu out;
while (renoir_lean_try_read(sub, (uint8_t *)&out, sizeof out) == RenoirErrorCode_Success) {
}
renoir_lean_close(sub);
renoir_lean_close(pub);
```

C++ callers use `renoir::LeanChannel` from `renoir_lean.hpp`.

## Contract

- One publisher per ring. The publisher never blocks, never allocates, makes no
  syscall and takes no lock. A second concurrent writer corrupts ordering.
- A handle is used by one thread at a time. Open one handle per thread.
- Readers are lossy. A reader that falls more than the slot count behind skips to
  a recent message and counts the loss (`renoir_lean_lost`). Use
  `renoir_lean_skip_to_latest` to always process the freshest sample.
- Reads are seqlock-validated: a torn slot is never returned.
- Message size and slot count must match across all openers of a name. A mismatch
  returns `InvalidParameter`.
- There is no wake-up primitive. Consumers poll. Blocking wake-up costs about 10 us
  and belongs on the broadcast topic API, not here.
- Frames larger than 504 bytes (camera images) use the broadcast topic path.

## Measured latency (x86, i7-11800H, pinned physical cores, one-way)

| Path | One-way p50 |
|---|---|
| Raw cache-line handoff | 66 ns |
| Lean ring, Rust API | 65-67 ns |
| Lean ring, C ABI | 71-76 ns |
| Broadcast topic ring | 250 ns |

Run `cargo bench --bench latency_pingpong -- <cpuA> <cpuB>`. Tail latency is set by the
operating system, not the ring. For bounded p99.9 on the target use `isolcpus`,
`nohz_full`, IRQ affinity and a `SCHED_FIFO` consumer, and pin both threads to
separate physical cores. On the Jetson, nanosecond timing needs the PMU cycle
counter because the generic timer ticks at about 31 MHz.

## Verification

Unit tests, loom model of the publish and read protocol, Miri (default and Tree
Borrows), ThreadSanitizer, AddressSanitizer, a corrupt-memory fuzz target, a counting
allocator test that proves zero allocation per publish and read, and an aarch64 run
under QEMU. See `.github/workflows/verify.yml`.
