# Releasing Renoir

1. Start from a clean `main` and a branch `release/x.y.z`.
2. Bump the version in `Cargo.toml` and `renoir-python/Cargo.toml`, move the `Unreleased` section of `CHANGELOG.md` under the new version, and follow `SEMVER.md` for the bump size (a wire-format or C ABI break needs a major bump).
3. Run the gate:
   - `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --release`
   - Miri: `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test` on the ring suites, then again with `-Zmiri-tree-borrows -Zmiri-symbolic-alignment-check`
   - sanitizers: `RUSTFLAGS=-Zsanitizer=thread` and `address` with `-Zbuild-std --target x86_64-unknown-linux-gnu`
   - loom: `RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 cargo test --release --test lean_loom`
   - memory model: `tests/litmus/run.sh` (needs `herd7`)
   - C and C++: `tests/c/run.sh`, `tests/abi/check.sh`, `tests/abi/abidiff.sh`, cmake build and `ctest`
   - Python: `maturin develop --release && pytest renoir-python/tests`
   - supply chain: `cargo deny check` and `cargo audit`
4. Reproducibility and SBOM: `scripts/repro_build.sh` must print `REPRODUCIBLE`; `scripts/sbom.sh` writes `dist/renoir.cdx.json`.
5. Build artifacts: `scripts/package_jetson.sh` (aarch64 tarball), `cargo deb` (x86_64 package), the Python wheel with `maturin build --release`.
6. Write `dist/SHA256SUMS` for every artifact and sign it with the project key (`minisign -S -m dist/SHA256SUMS`); publish the public key in the repository.
7. Soak: the Jetson soak (`scripts/jetson_soak.sh`) for at least 24 hours with no loss on gated topics and no growth in `lost` beyond the documented budget.
8. Open a pull request from the release branch, get it reviewed, merge, then tag `vx.y.z` on the merge commit. Never force-push a tag.
