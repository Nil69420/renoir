# Performance

## Baseline

Renoir's own benchmark suite measures the actual topic pub/sub hot path
(the `topic_rings` Criterion benchmark). The baseline is recorded in
`.ai/benchmarks/topic_rings_baseline.md` with the methodology for
reproducing it.

## Comparison vs iceoryx2

A same-hardware, same-operation comparison against iceoryx2 0.9.3 was
run (256-byte payload, 100k publish/receive round trips, 3 rounds).
Full methodology and reproduction steps are in
`.ai/benchmarks/iceoryx2_comparison.md`.

**Result — iceoryx2 is faster in this benchmark.** Renoir loses by
roughly 1.8x on mean latency and throughput:

| Metric | Renoir 0.1.0 | iceoryx2 0.9.3 |
|---|---|---|
| Mean latency | 456–529 ns | 256–295 ns |
| p50 | 441–487 ns | 248–278 ns |
| p99 | 725–854 ns | 440–539 ns |
| Throughput | 1.75–2.01 M msg/s | 2.90–3.38 M msg/s |

### Why (honest analysis)

- iceoryx2 loans zero-copy samples on both send and receive; Renoir's
  inline ring path copies into the ring on publish and copies out into a
  fresh buffer on subscribe.
- Renoir computes a CRC32 checksum per inline message (a safety feature
  iceoryx2 does not have); that is part of the measured cost.
- Renoir's README "zero-copy" claim is currently true only for the
  shared-pool/descriptor path, not the inline ring path measured here.

This is a real gap and a real target: the fastest credible next step is a
zero-copy receive path for inline ring messages, after which the
comparison should be re-run and this page updated.
