# Formal models

`LeanMP.tla` models the multi-producer protocol of the lean ring: claim, mark, slot acquire, takeover of a stuck slot, payload write, commit, producer crash, and the reader's read and skip, with gating. It is checked with TLC (the free, open-source TLA+ model checker) by `run.sh`.

Properties:
- `NoTornRead`: a reader that sees a stable slot always sees the payload of that slot's index.
- `NoLossOfLiveProducer`: the reader only skips an index whose owner is dead.

`MODE = "OLD"` is the timer rule shipped before the producer table (skip or take over whenever a slot has been unpublished for a while). TLC finds counterexamples for both properties, which shows the model captures the real failure. `MODE = "NEW"` is the liveness rule (skip or take over only when no live producer covers the index, where "covers" includes the claiming marker set before the claim). TLC finds no violation in the configurations in `run.sh` (up to 4 producers, 3 slots, 7 indices, which is 18.6 million distinct states, and 3 producers, 4 slots, 8 indices).

What the model does not cover:
- Timers: the 10 s hard-stall rule and the timer fallback for producers that could not register are not modelled, so the proof is for the registered, live-or-dead case only.
- Memory ordering: the model is sequentially consistent. Weak-memory behaviour is covered separately by the herd7 litmus tests in `tests/litmus` and the native arm64 CI job.
- Bounded size: results hold for the checked bounds, not for all sizes.
- The code itself: the model is hand-written from the Rust, not extracted from it. Keep them in step when the protocol changes.

`FrameGate.tla` models the frame ring ownership protocol: the producer admission rule, consumers viewing and releasing, consumer death, and eviction. The property is `NoOverwriteWhileHeld`: no slot is overwritten while a consumer that is not dead still holds it. `FrameGate_Timer.cfg` lets the gate evict a live holder, as the lean ring's timer rule does, and TLC finds the violation; `FrameGate_Death.cfg` evicts only dead holders and TLC finds none (up to 3 slots, 3 consumers, 7 frames, 85,256 distinct states). It does not model the pid-reuse or pid-namespace limits, weak memory, or the multi-producer claim path (see `LeanMP.tla`).

`StampedSeqlock.tla` models a stamped lean ring slot: the writer sets the sequence odd, writes the stamp, writes the payload, then sets the sequence even; the reader checks the sequence, reads stamp and payload as separate steps, and re-checks the sequence. `NoMixedMessage` says an accepted read never returns a stamp and payload from different messages. `StampedSeqlock_NoRecheck.cfg` removes the final re-check and TLC finds the violation, so the model does capture the hazard; with the re-check TLC finds none. It is sequentially consistent, so it checks the protocol, not the memory ordering (the fences and acquire/release pairs are covered by the herd7 tests, Miri, loom and the arm64 job), and it models one producer; the stamp adds no new protocol step for multi-producer rings, which write it inside the same claimed slot.

## Kani

`src/topic_rings/lean/proofs.rs` holds Kani harnesses (run with `cargo kani --lib`, compiled only under `cfg(kani)`). They check the sizing and offset arithmetic that feeds the unsafe pointer math: slot sizes are line aligned and bounded for every possible size, slot counts are powers of two that fit the ring, a slot offset never leaves the slot area, and header inspection rejects short buffers. They check these formulas, not the whole unsafe code; Miri, the sanitizers and the fuzzers cover that. The first run found a real overflow in `lean_slot_bytes`, now fixed.

The stamp work added six more: the stamped slot always holds sequence, stamp and payload and agrees with the unstamped sizing; the clock mode decoding round-trips and rejects unknown values; sequence numbers keep their odd/even meaning below index 2^62 (above it `2 * idx` overflows, which at one million messages a second is about 146,000 years); the tick-to-nanosecond product cannot overflow for any counter value; and the conversion is monotonic and exact for whole seconds. The last one is proved only over a bounded range (rates up to 63 Hz, up to 255 ticks) because CBMC does not finish a fully symbolic 128-bit division (two attempts timed out at 100 s and 250 s, and a different SAT solver did not help); the real-rate range is covered by a 100,000-sample monotonicity test (`tests/lean_stamped_tests.rs`), which is evidence rather than proof. Changing the conversion constant makes the bounded proof fail, which shows the harness is checking something.

## Weak memory at code level

`examples/genmc_stamped_seqlock.rs` runs the real `LeanRing` publish and read on two threads over a two-slot ring, so the writer laps the reader, and asserts that an accepted read never mixes data from two messages. Arguments are the number of messages, the number of reader polls, and `0` for the stamped ring or `1` for the plain one (stamp and payload must match, or the two halves of the payload must match).

**GenMC** (the stateless model checker built into Miri) explores every execution the Rust memory model allows, including every thread interleaving and every value an atomic load may return. Measured with Miri master and GenMC at the commit that Miri pins, LLVM 20.1.8:

| Ring | Messages | Reader polls | Executions | Result | Time |
|---|---|---|---|---|---|
| stamped | 3 | 1 | 1,142 | no errors | 89 s |
| stamped | 3 | 2 | 2,516 | no errors | 182 s |
| stamped | 3 | 5 | 12,238 | no errors | 917 s |
| plain | 3 | 1 | 1,112 | no errors | 82 s |
| plain | 3 | 2 | 2,466 | no errors | 184 s |

At 2 messages the writer never laps the reader, so nothing can go wrong and a run proves nothing: all three mutants below also "passed" at that bound, which is why the bound is 3. Three deliberate mutations of the stamped path, each run at 3 messages and 1 poll, are all rejected with "stamp and payload from different messages": removing the writer's release fence, removing the reader's acquire fence, and removing the reader's final sequence re-check. So GenMC does detect ordering bugs in this code, not just the sequential kind.

What this covers and what it does not:
- It covers the plain-ring and stamped seqlock fast paths in single-producer mode, bounded at 3 messages and up to 5 polls. It is exhaustive for that program, not for all message counts; for those the TLA+ model and the Kani proofs apply.
- Multi-producer and gated rings are not covered: they register process ids through `/proc` and use timers and futexes, which GenMC cannot model. They are covered by TLA+, loom, shuttle, herd7 and the chaos tests.
- GenMC support in Miri is upstream work in progress with documented correctness limits, and it runs with borrow tracking off (`-Zmiri-disable-stacked-borrows`), so aliasing is checked separately by the regular Miri runs.
- The reader does not loop forever on a stuck slot in this harness (polls are bounded), as GenMC requires.

To reproduce, you need Miri built from source with the `genmc` feature. On this Ubuntu 22.04 machine the system compiler was too old (GenMC uses C++20 `std::format`, which needs GCC 13 or newer headers) and no root was available, so the setup used user-level tools: the LLVM 20.1.8 release tarball for LLVM and clang; GCC 14 headers and libstdc++ from conda-forge through micromamba; `rustup-toolchain-install-master` and `./miri toolchain` for the exact rustc; then in the Miri checkout, with `CC` and `CXX` set to the LLVM clang, `CFLAGS` and `CXXFLAGS` containing `--gcc-install-dir=/usr/lib/gcc/x86_64-linux-gnu/11 -nostdinc++ -isystem <gcc14 include/c++> -isystem <gcc14 include/c++/x86_64-conda-linux-gnu>`, `LDFLAGS` and `RUSTFLAGS` linking and rpath-ing the conda `lib`, run `./miri install --features=genmc`. That installs into the `miri` rustup toolchain only. Then:

```sh
MIRIFLAGS="-Zmiri-genmc -Zmiri-disable-stacked-borrows -Zmiri-genmc-verbose" \
  cargo +miri miri run --example genmc_stamped_seqlock -- 3 1 0
```

**Random schedules** (regular Miri, any machine with nightly): `MIRIFLAGS="-Zmiri-many-seeds=0..400 -Zmiri-preemption-rate=0.5 -Zmiri-disable-isolation" cargo +nightly miri run --example genmc_stamped_seqlock -- 12 96 0`. This finds bugs but cannot prove their absence; removing the re-check makes it fail at 12 messages and not at 3.
