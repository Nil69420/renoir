# Production readiness: what is verified and what is not

This page lists the evidence behind Renoir's claims and its known limits, so you can decide what to rely on. Everything listed under "verified" was run by the project's own tooling; none of it replaces a review of your own deployment. All tools are free and open source.

## Verified

| Claim | Evidence |
|---|---|
| No undefined behaviour in the ring code on the tested paths | Miri (default and Tree Borrows with symbolic alignment) on the lean, gated, multi-producer, batch, broadcast-view and init-race suites |
| No data races | ThreadSanitizer and loom (5 models) and shuttle (2 scenarios) on the seqlock, multi-producer and futex handshakes |
| No memory errors | AddressSanitizer, valgrind memcheck on every C test, UBSan on the C tests |
| Correct on weak memory | herd7 litmus tests against the AArch64 model (4 protocol tests that must never show the bad outcome, 4 negative controls that must), and a native arm64 CI job |
| Multi-producer protocol | TLA+ model (`formal/LeanMP.tla`): no torn read and no lost message from a live producer under the liveness rule; the old timer rule fails both |
| Frame ownership | TLA+ model (`formal/FrameGate.tla`): no slot overwritten while a live consumer holds it; evicting live holders by timer fails |
| Sizing arithmetic | Kani proofs (`src/topic_rings/lean/proofs.rs`); the first run found and fixed a real overflow |
| Corrupt shared memory cannot crash readers | cargo-fuzz targets for lean, broadcast and frame rings (60-second runs: 0.6 to 3.5 million executions per target, no crash), plus corrupt-header unit tests |
| Crash recovery | Chaos tests that SIGKILL publishers, consumers, frame holders, frame loaners and RPC servers while traffic runs |
| No allocation, lock, syscall or panic on the hot paths | A counting-allocator test (steady state, all lean modes, broadcast view, RPC) and `objdump` review of the publish and read fast paths |
| Latency | Pinned-core ping-pong benchmarks, published with method and caveats in `docs/src/benchmarks.md` |
| ABI stability | SONAME `librenoir.so.0`, an exported-symbol baseline check and a libabigail comparison against a checked-in baseline in CI |
| Supply chain | `cargo deny`, `cargo audit`, an SBOM, and a reproducible `librenoir.so` build |

## Not verified, or only partly

- **No Jetson numbers.** All latency figures are from an x86 laptop (i7-11800H). The Jetson soak (24 hours or more) and Jetson timing have not been run by the project.
- **Weak memory is model-checked, not exhaustively tested on hardware.** At code level, GenMC (Miri's exhaustive weak-memory model checker) explored every execution of the single-producer plain and stamped seqlock fast paths at 3 messages through a 2-slot ring with up to 5 reader polls (12,238 executions for the largest stamped case) and found no errors; removing the writer's release fence, the reader's acquire fence or the sequence re-check each makes it fail. That covers those paths at those bounds only: multi-producer and gated rings use `/proc` and futexes that GenMC cannot model, and GenMC support in Miri is upstream work in progress (see `formal/README.md`). herd7 covers the protocols and one native arm64 CI job runs the stress tests; no multi-day ARM soak exists.
- **The models are hand-written.** The TLA+ and Kani artefacts check the protocol and formulas, not the Rust itself; they must be kept in step with the code. The TLA+ results hold for small bounds (a few slots, three producers, a few consumers).
- **The C surface has not been audited for MISRA or CERT compliance, and no compliance claim is made.** `scripts/c_surface_check.sh` runs cppcheck's MISRA C:2012 addon (rule numbers only) and clang-tidy's CERT, bugprone and analyzer checks on the C tests, the soak tool and the shipped header. Last run: the tests produce about 250 clang-tidy warnings (mostly CERT ERR33-C, unchecked return values in assertion macros) and the shipped header 2 MISRA lines with the script's settings (rules 3.1 and 18.5). With the stricter `--enable=all --inconclusive` the header gives 132 lines, 125 of them rules 8.7 and 2.5 (external declarations and macros that exist because they are the public interface, so deviations rather than defects), plus 2.3 (5 unused type declarations), 2.4 and 18.5. Two real findings in shipped tooling were fixed: the soak tool leaked a subscriber and buffer when an allocation failed, and parsed numeric options with `atoi` (a negative `--width` wrapped to a huge value and passed validation). The generated header has not been reviewed rule by rule, and the Rust code has no MISRA-style analysis. There has been no third-party security review.
- **Liveness uses process ids.** Processes sharing a ring must share a pid namespace and must be able to see each other's `/proc` entry.
- **Third-party workflows are new.** The CodeQL, OpenSSF Scorecard and Kani CI jobs are non-blocking until they have been seen running on this repository. The OSS-Fuzz files in `ossfuzz/` have not been submitted.
- **Single-producer lean rings assume one writer.** A second writer is a protocol violation; it can corrupt ordering and contents but not readers' memory safety.
- **Limits by design:** at most 504 bytes per lean message, 64 MiB per frame, 32 gated consumers, 16 registered publishers and 4 RPC servers per service. RPC failover can deliver a request twice, so handlers should be idempotent.

## Before depending on it

Run the soak on your hardware (`scripts/jetson_soak.sh`), run `scripts/ci_local.sh` from a clean checkout, and check `docs/src/troubleshooting.md` for the failure modes you should alarm on.
