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
| Corrupt shared memory cannot crash readers | cargo-fuzz targets for lean, broadcast and frame rings (millions of executions per target, no crash), plus corrupt-header unit tests |
| Crash recovery | Chaos tests that SIGKILL publishers, consumers, frame holders, frame loaners and RPC servers while traffic runs |
| No allocation, lock, syscall or panic on the hot paths | A counting-allocator test (steady state, all lean modes, broadcast view, RPC) and `objdump` review of the publish and read fast paths |
| Latency | Criterion-free pinned ping-pong benchmarks, published with method and caveats in `docs/src/benchmarks.md` |
| ABI stability | SONAME `librenoir.so.0`, an exported-symbol baseline check and a libabigail comparison against a checked-in baseline in CI |
| Supply chain | `cargo deny`, `cargo audit`, an SBOM, and a reproducible `librenoir.so` build |

## Not verified, or only partly

- **No Jetson numbers.** All latency figures are from an x86 laptop (i7-11800H). The Jetson soak (24 hours or more) and Jetson timing have not been run by the project.
- **Weak memory is model-checked, not exhaustively tested on hardware.** herd7 covers the protocols and one native arm64 CI job runs the stress tests; no multi-day ARM soak exists.
- **The models are hand-written.** The TLA+ and Kani artefacts check the protocol and formulas, not the Rust itself; they must be kept in step with the code. The TLA+ results hold for small bounds (a few slots, three producers, a few consumers).
- **The C surface has not been audited for MISRA or CERT compliance.** `scripts/c_surface_check.sh` runs cppcheck's MISRA addon (rule numbers only) and clang-tidy's CERT checks on the C test code; findings are in the test harness, none in shipped code, and no compliance claim is made. There has been no third-party security review.
- **Liveness uses process ids.** Processes sharing a ring must share a pid namespace and must be able to see each other's `/proc` entry.
- **Third-party workflows are new.** The CodeQL, OpenSSF Scorecard and Kani CI jobs are non-blocking until they have been seen running on this repository. The OSS-Fuzz files in `ossfuzz/` have not been submitted.
- **Single-producer lean rings assume one writer.** A second writer is a protocol violation; it can corrupt ordering and contents but not readers' memory safety.
- **Limits by design:** at most 504 bytes per lean message, 64 MiB per frame, 32 gated consumers, 16 registered publishers and 4 RPC servers per service. RPC failover can deliver a request twice, so handlers should be idempotent.

## Before depending on it

Run the soak on your hardware (`scripts/jetson_soak.sh`), run `scripts/ci_local.sh` from a clean checkout, and check `docs/src/troubleshooting.md` for the failure modes you should alarm on.
