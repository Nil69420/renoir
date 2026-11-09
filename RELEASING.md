# Releasing Renoir

1. Start from a clean `main` and a branch `release/x.y.z`.
2. Bump the version in `Cargo.toml` and `renoir-python/Cargo.toml`, move the `Unreleased` section of `CHANGELOG.md` under the new version, and follow `SEMVER.md` for the bump size (a wire-format or C ABI break needs a major bump).
3. Run the gate (`scripts/ci_local.sh` runs all of the first block in one go, including Miri, both sanitizers, the TLA+ models and Kani when installed; run it before every push):
   - `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --profile testing` (no LTO, fast to build; the shipped `release` profile is unchanged)
   - formal models: `formal/run.sh` (TLC, needs Java) and `cargo kani --lib`
   - Miri: `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test` on the ring suites, then again with `-Zmiri-tree-borrows -Zmiri-symbolic-alignment-check`
   - sanitizers: `RUSTFLAGS=-Zsanitizer=thread` and `address` with `-Zbuild-std --target x86_64-unknown-linux-gnu`
   - loom: `RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 cargo test --release --test lean_loom`
   - memory model: `tests/litmus/run.sh` (needs `herd7`)
   - C and C++: `tests/c/run.sh`, `tests/abi/check.sh`, `tests/abi/abidiff.sh`, cmake build and `ctest`
   - Python: `maturin develop --release && pytest renoir-python/tests`
   - supply chain: `cargo deny check` and `cargo audit`
4. Reproducibility and SBOM: `scripts/repro_build.sh` must print `REPRODUCIBLE`; `scripts/sbom.sh` writes `dist/renoir.cdx.json`.
5. Build artifacts: `scripts/package_jetson.sh` (aarch64 tarball), `cargo deb` (x86_64 package), the Python wheel with `maturin build --release`.
6. Signing is done by the `Release` workflow when the tag `vx.y.z` is pushed (step 8): it rebuilds the artifacts, checks the tag against the crate version and the reproducibility script, writes `SHA256SUMS`, attaches a Sigstore build-provenance attestation to every artifact and the checksum file, attests the SBOM, signs `SHA256SUMS` with cosign (keyless, bound to the workflow identity, so there is no private key to store or lose) and opens a draft GitHub release for review. Consumers verify with:
   - `gh attestation verify <artifact> --repo Nil69420/renoir`
   - `cosign verify-blob --certificate SHA256SUMS.pem --signature SHA256SUMS.sig --certificate-identity-regexp 'https://github.com/Nil69420/renoir/.github/workflows/release.yml@refs/tags/v.*' --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS`, then `sha256sum -c SHA256SUMS`
   The workflow has not run yet; the first release candidate tag is its first test.
7. Soak: the Jetson soak (`scripts/jetson_soak.sh`) for at least 24 hours with no loss on gated topics and no growth in `lost` beyond the documented budget.
8. Open a pull request from the release branch, get it reviewed, merge, then tag `vx.y.z` on the merge commit. Never force-push a tag.
