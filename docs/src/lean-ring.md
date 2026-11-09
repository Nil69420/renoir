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

- One publisher per ring unless it is opened in multi-producer mode (see below).
  `publish` never blocks, never allocates, makes no syscall and takes no lock; on a
  gated ring it returns `BufferFull` instead of overwriting unread messages. A second
  concurrent writer on a single-producer ring corrupts ordering.
- A handle is used by one thread at a time. Open one handle per thread.
- By default readers are lossy. A reader that falls more than the slot count behind
  skips to a recent message and counts the loss (`renoir_lean_lost`). Use
  `renoir_lean_skip_to_latest` to always process the freshest sample, or open the
  ring gated for lossless delivery (see below).
- Reads are seqlock-validated: a torn slot is never returned.
- Message size and slot count must match across all openers of a name. A mismatch
  returns `InvalidParameter`.
- Consumers poll by default. A blocking wait with a futex wake-up (about 10 us)
  is available through `publish_notify` and `wait`; the 100 ns path is the polling one.
- Messages larger than 504 bytes (camera images) use a frame ring (see frames.md).

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

## Publish timestamps

A sensor message is only useful for estimation if the consumer knows when it was produced, so a lean ring can carry an 8-byte publish stamp inside each slot, written and read under the same sequence check as the payload. Open the ring with `RENOIR_LEAN_STAMP_NS` (nanoseconds on `CLOCK_MONOTONIC`) or `RENOIR_LEAN_STAMP_TICKS` (the raw cycle counter: `rdtsc` on x86-64, `cntvct_el0` on AArch64). `msg_size` stays the size of your data; the ring adds the stamp, so a one-line slot carries 48 data bytes instead of 56 and the largest stamped message is 496 bytes. Use `renoir_lean_memory_required_flags` to size caller-provided memory.

```c
renoir_lean_open("imu", sizeof(struct imu), 1024, RENOIR_LEAN_STAMP_TICKS, &pub);
renoir_lean_publish_now(pub, (const uint8_t *)&sample, sizeof sample);   /* stamp from the ring clock */
renoir_lean_publish_at(pub, hardware_timestamp, (const uint8_t *)&sample, sizeof sample);

uint64_t stamp;
if (renoir_lean_try_read_stamped(sub, (uint8_t *)&out, sizeof out, &stamp) == RenoirErrorCode_Success) {
    uint64_t ns = renoir_lean_stamp_to_ns(sub, stamp);
}
```

`publish_at_notify` and `publish_now_notify` do the same and also wake consumers blocked in the Blocking or PhasedBackoff waits (plain `publish_at` and `publish_now` do not, so a blocked consumer would sleep until its timeout). `publish_at` takes a stamp you already have, such as the sensor's own hardware time, and costs no clock read at all. Use it when the driver supplies the time of measurement, which is more accurate than the time of publication. The two clock choices differ in cost, not in meaning:

| Publish (48 B, one core, i7-11800H) | Cost over an unstamped publish |
|---|---|
| `publish_at`, caller-supplied stamp | about 2 ns |
| `publish_now`, tick clock | about 4 ns |
| `publish_now`, nanosecond clock | about 14 ns (a `clock_gettime` call through the vDSO) |

Tick stamps are comparable between processes on one machine because the counter is invariant and shared. Convert them with the ring's tick rate (`renoir_lean_tick_hz`), which is read from CPUID leaf 0x15 or calibrated against `CLOCK_MONOTONIC` when the CPU does not report it; if the CPU has no invariant counter, opening a tick ring fails and you should use the nanosecond clock. On AArch64 the rate is `cntfrq_el0`, typically a coarse clock (31.25 MHz on some Jetson parts is 32 ns per tick); the project has not measured Jetson. The ring remembers its clock in the header, so a process opening it with a different clock mode is rejected. The unstamped `publish` and `try_read` are not meant for a stamped ring: they work on the wire format, so a buffer of your data size plus eight bytes is accepted and its first eight bytes are the stamp, and any other size returns an error. Use the stamped calls.

Stamped reads on plain rings take a direct path; gated and multi-producer rings go through a stack buffer and a copy. No path allocates (`tests/zero_alloc_paths.rs`).

## Reporting what was missed

A lossy ring overwrites the oldest message when a reader falls behind. `renoir_lean_lost` gives the running total; `renoir_lean_try_read_lagged` and `renoir_lean_try_read_stamped_lagged` (Rust: `try_read_lagged`, `try_read_stamped_lagged`) return the number of messages missed by that call as well, so a consumer can react at the moment the gap appears (for example, reset a filter when a sample is missing instead of integrating across the hole). Messages read plus messages missed always equals messages published since the reader started.

To always work on the newest value, call `renoir_lean_skip_to_latest(handle, 1)` and then read: the skipped messages are counted as lost, and the read returns the latest one.

## Blocking consumers

`publish` never issues a fence or a syscall beyond the seqlock stores, so a spinning consumer sees data in about the cache-line transfer time. A consumer that cannot afford to spin can block instead:

- Publisher: call `publish_notify` (`renoir_lean_publish_notify`, `publishNotify`, `publish_notify`). It publishes, issues one sequentially consistent fence (about 3 ns on x86 when nobody is waiting) and wakes waiters through a futex only when a waiter has registered.
- Consumer: call `wait` (`renoir_lean_wait`, `wait`). It spins for `spin_iterations` checks, then registers as a waiter and sleeps on the futex until data arrives or the timeout expires. A timeout of `u64::MAX` nanoseconds in C means wait forever.

A publisher that uses plain `publish` does not wake sleeping consumers; a consumer that calls `wait` with a finite timeout and `spin_iterations` large enough for its latency budget still works against either publisher.

The store-buffering handshake between `publish_notify` and `wait` is checked against the AArch64 memory model with `herd7` (`tests/litmus/wake_handshake.litmus`, plus a negative control with the fence removed).

## Multiple producers

Open the channel with the multi-producer flag (`RENOIR_LEAN_MULTI_PRODUCER`, `multiProducer`, `multi_producer=True`, `LeanMode::MultiProducer`) and any number of threads or processes may publish to it. The mode is stored in the ring header and a mismatching open is rejected, so single- and multi-producer publishers can never share a ring.

Each publish claims a sequence number with an atomic add, takes a per-slot ticket with a compare-and-swap, copies the payload and publishes. An uncontended publish costs about 26 ns against about 7 ns for the single-producer path, and readers pay nothing extra on the hot path.

Failure behaviour, which differs from the single-producer ring:

- Multi-producer publishers register in a 16-entry producer table in the ring header (process id and process start time, plus the index range they currently have claimed). A producer killed between claiming and publishing leaves a hole. A reader that polls an unpublished slot while a later slot has been claimed waits 5 ms, then asks whether the claimant is alive. A dead claimant (process gone, or pid reused by another process) is skipped at once and counted in `lost`. A claimant that is alive, for example suspended by SIGSTOP or a debugger, is waited for, so no message is lost and no payload is torn; after 10 s of stall it is treated as dead.
- A producer that cannot register (more than 16 live publishers on one ring, or no pid namespace shared with the other processes) falls back to the older timer rule: skipped after 5 ms on a plain ring, after the gate timeout on a gated ring, and its slot can be taken over after 50 ms. Keep producers in one pid namespace; containers that share the shared-memory directory must share the pid namespace too.

## Gated lossless mode

By default a lean ring is lossy: a slow reader is lapped and counts `lost`. Open the ring with the gated flag (`RENOIR_LEAN_GATED`, `LeanFlags::Gated`, `gated=True`, `LeanMode::SingleProducerGated`) and register each consumer that must not lose messages (`RENOIR_LEAN_CONSUMER`, `LeanFlags::Consumer`, `consumer=True`, `register_consumer`). A publisher on a gated ring never overwrites a message that a registered consumer has not read: when the ring is full, `publish` returns `BufferFull` (`RenoirErrorCode_BufferFull`) and the caller decides to spin, retry or drop. Unregistered readers on the same ring stay lossy.

Up to 8 consumers can be registered per ring. Each registered reader stores its position after every read; the publisher keeps a cached minimum and only scans the table when the ring is nearly full.

A dead or stuck consumer must not stop the producers forever. If the slowest registered consumer has not advanced for the gate timeout (100 ms by default, `LeanRing::set_gate_timeout`), the publisher evicts it. An evicted consumer notices on its next read, re-registers at its current position and resumes; whatever the publisher overwrote in the meantime is reported through `lost`. Lossless delivery therefore holds for consumers that make progress within the timeout.

On a gated multi-producer ring (`MultiProducerGated`) producers claim indices with a compare-and-swap that checks the gate first, so the ring never claims a slot it cannot publish.

Read cost: one extra release store per read for registered consumers; unregistered readers pay nothing.

## Batching

`publish_batch` (`renoir_lean_publish_batch`, `publishBatch`, `publish_batch`) writes several messages given as one contiguous buffer whose length is a multiple of the message size, and `read_batch` (`renoir_lean_read_batch`, `readBatch`, `read_batch`) reads up to as many as fit in the output buffer. Every message keeps its own sequence word, so readers still see messages one by one and in order; a batch is not atomic.

Savings measured on x86 for eight 32-byte messages: 28.6 ns as one batch against 51.8 ns as eight publishes on a single-producer ring, and 170 ns against 212 ns on a multi-producer ring, where the batch claims all eight indices with one atomic add. A gated ring publishes as many messages of the batch as fit and reports the count; if none fit, `publish_batch` returns `BufferFull`. `read_batch` returns zero when nothing is available and `renoir_lean_read_batch` reports that with `RenoirErrorCode_Success` and a count of zero. On a gated ring a batch read updates the consumer position once per batch.

## Adaptive waiting

`wait_adaptive` (`renoir_lean_wait_adaptive`, `waitAdaptive`, `wait_adaptive`) tunes the spin budget itself. It starts at 1000 iterations, doubles the budget (up to 100,000) each time data arrives while still spinning, and halves it (down to 32) each time it has to fall through to the futex. Frequent messages therefore keep consumers on the polling path with no syscall, and sparse streams stop burning CPU. Use `wait` with an explicit spin count when you need a fixed budget.


## Wait strategies

A consumer that has nothing to read has to wait somehow, and the right choice trades latency against CPU. `renoir_lean_wait_strategy(handle, strategy, timeout_ns)` (Rust: `LeanChannel::wait_with`) offers the set the LMAX Disruptor uses; every one takes a timeout (`UINT64_MAX` for none), which also gives the Disruptor's timeout variants.

| Strategy | Behaviour | Latency, CPU | Publisher must call |
|---|---|---|---|
| `RENOIR_LEAN_WAIT_BUSY_SPIN` | poll with a pause instruction | lowest latency, one core at 100% | `publish` |
| `RENOIR_LEAN_WAIT_YIELDING` | poll and `sched_yield` between polls | low latency, core shared with other runnable threads | `publish` |
| `RENOIR_LEAN_WAIT_SLEEPING` | poll, then sleep 50 us per step | latency up to about 50 us plus the scheduler's timer slack, almost no CPU | `publish` |
| `RENOIR_LEAN_WAIT_BLOCKING` | futex wait, no polling | a wake-up through the kernel (microseconds), no CPU while idle | `publish_notify`, or `publish_at_notify` / `publish_now_notify` on a stamped ring |
| `RENOIR_LEAN_WAIT_PHASED_BACKOFF` | spin for 50 us, yield for 1 ms, then block | spin latency for dense streams, no CPU for sparse ones | `publish_notify` |
| `renoir_lean_wait_phased(handle, spin_ns, yield_ns, timeout_ns)` | the same with your own phase lengths | | `publish_notify` |

`renoir_lean_wait_adaptive` (below) is a seventh option that tunes its own spin budget. The blocking phases only wake if the publisher uses `publish_notify`, which skips the wake-up system call when no consumer is blocked (the Disruptor's "lite" blocking optimisation); the polling strategies need no notification. These strategies are tested for correct timeout and wake-up, not benchmarked for latency here, so measure on your own hardware, particularly the sleeping and yielding ones, whose behaviour depends on the kernel's scheduler.

## Crash recovery

- A publisher killed mid-publish leaves an odd sequence word in one slot. Readers see that slot as empty; the next publisher (single-producer) resumes at the published index and rewrites it, so the ring heals without intervention.
- A process killed while initialising a ring leaves a half-initialised header. If nothing else holds the region, the next opener wipes and rebuilds it. If other processes already hold it, the next opener waits 2 seconds and then takes over the initialisation.
- A consumer killed while registered on a gated ring stalls publishers only until the gate timeout (100 ms by default) evicts it.
