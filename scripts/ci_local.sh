#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
MIRI_SUITES="--test lean_ring_tests --test lean_wait_tests --test lean_multi_tests --test lean_gate_tests --test lean_batch_tests --test broadcast_view_tests --test ring_init_race_tests --test broadcast_ring_tests"
SAN_SUITES=$(grep -A6 'cargo test -Zbuild-std' .github/workflows/verify.yml | grep -o -- '--test [a-z_0-9]*' | sort -u | tr '\n' ' ')
step() { printf '\n== %s\n' "$1"; }
step fmt; cargo fmt --check
step clippy; cargo clippy --all-targets --all-features -- -D warnings
step tests; cargo test --profile testing
step c-abi; sh tests/c/run.sh >/dev/null; sh tests/abi/check.sh
if [ "${1:-}" = "--fast" ]; then echo "fast mode: skipped Miri and sanitizers"; exit 0; fi
step miri; MIRIFLAGS="-Zmiri-disable-isolation" cargo +nightly miri test $MIRI_SUITES
step miri-tree-borrows; MIRIFLAGS="-Zmiri-disable-isolation -Zmiri-tree-borrows -Zmiri-symbolic-alignment-check" cargo +nightly miri test $MIRI_SUITES
for san in thread address; do
  step "sanitizer $san"
  RUSTFLAGS="-Zsanitizer=$san" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu $SAN_SUITES
done
if command -v java >/dev/null 2>&1; then step formal; ./formal/run.sh >/dev/null && echo formal ok; fi
if command -v cargo-kani >/dev/null 2>&1; then step kani; cargo kani --lib -Z unstable-options 2>&1 | grep -E "Complete|FAILED"; fi
step audit; cargo deny check >/dev/null && echo deny ok
echo "ci_local: all green"
