# Formal models

`LeanMP.tla` models the multi-producer protocol of the lean ring: claim, mark, slot acquire, takeover of a stuck slot, payload write, commit, producer crash, and the reader's read and skip, with gating. It is checked with TLC (the free, open-source TLA+ model checker) by `run.sh`.

Properties:
- `NoTornRead`: a reader that sees a stable slot always sees the payload of that slot's index.
- `NoLossOfLiveProducer`: the reader only skips an index whose owner is dead.

`MODE = "OLD"` is the timer rule shipped before the producer table (skip or take over whenever a slot has been unpublished for a while). TLC finds counterexamples for both properties, which shows the model captures the real failure. `MODE = "NEW"` is the liveness rule (skip or take over only when no live producer covers the index, where "covers" includes the claiming marker set before the claim). TLC finds no violation in the configurations in `run.sh` (up to 3 producers, 3 slots, 6 indices).

What the model does not cover:
- Timers: the 10 s hard-stall rule and the timer fallback for producers that could not register are not modelled, so the proof is for the registered, live-or-dead case only.
- Memory ordering: the model is sequentially consistent. Weak-memory behaviour is covered separately by the herd7 litmus tests in `tests/litmus` and the native arm64 CI job.
- Bounded size: results hold for the checked bounds, not for all sizes.
- The code itself: the model is hand-written from the Rust, not extracted from it. Keep them in step when the protocol changes.
