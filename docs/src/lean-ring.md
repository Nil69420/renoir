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

- A producer killed between claiming and publishing leaves a hole. A reader that polls an unpublished slot while a later slot has been claimed waits 5 ms and then skips the hole, counting it in `lost`.
- A producer that stalls inside a publish for more than 50 ms can have its slot taken over by a producer that has wrapped the ring. The stalled producer's late completion fails with an error instead of publishing. Because the stalled producer's payload bytes may still land in the slot, this is a known limit: do not suspend producers (SIGSTOP, debugger) while they publish on a multi-producer ring.

## Gated lossless mode

By default a lean ring is lossy: a slow reader is lapped and counts `lost`. Open the ring with the gated flag (`RENOIR_LEAN_GATED`, `LeanFlags::Gated`, `gated=True`, `LeanMode::SingleProducerGated`) and register each consumer that must not lose messages (`RENOIR_LEAN_CONSUMER`, `LeanFlags::Consumer`, `consumer=True`, `register_consumer`). A publisher on a gated ring never overwrites a message that a registered consumer has not read: when the ring is full, `publish` returns `BufferFull` (`RenoirErrorCode_BufferFull`) and the caller decides to spin, retry or drop. Unregistered readers on the same ring stay lossy.

Up to 8 consumers can be registered per ring. Each registered reader stores its position after every read; the publisher keeps a cached minimum and only scans the table when the ring is nearly full.

A dead or stuck consumer must not stop the producers forever. If the slowest registered consumer has not advanced for the gate timeout (100 ms by default, `LeanRing::set_gate_timeout`), the publisher evicts it. An evicted consumer notices on its next read, re-registers at its current position and resumes; whatever the publisher overwrote in the meantime is reported through `lost`. Lossless delivery therefore holds for consumers that make progress within the timeout.

On a gated multi-producer ring (`MultiProducerGated`) producers claim indices with a compare-and-swap that checks the gate first, so the ring never claims a slot it cannot publish.

Read cost: one extra release store per read for registered consumers; unregistered readers pay nothing.

## Batching

`publish_batch` (`renoir_lean_publish_batch`, `publishBatch`, `publish_batch`) writes several messages given as one contiguous buffer whose length is a multiple of the message size, and `read_batch` (`renoir_lean_read_batch`, `readBatch`, `read_batch`) reads up to as many as fit in the output buffer. Every message keeps its own sequence word, so readers still see messages one by one and in order; a batch is not atomic.

Savings measured on x86 for eight 32-byte messages: 28.6 ns as one batch against 51.8 ns as eight publishes on a single-producer ring, and 170 ns against 212 ns on a multi-producer ring, where the batch claims all eight indices with one atomic add. A gated ring publishes as many messages of the batch as fit and reports the count; if none fit, `publish_batch` returns `BufferFull`. `read_batch` returns zero when nothing is available and `renoir_lean_read_batch` reports that with `RenoirErrorCode_Success` and a count of zero. On a gated ring a batch read updates the consumer position once per batch.
