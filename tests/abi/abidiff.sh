#!/usr/bin/env bash
set -uo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
tgt="${TMPDIR:-/tmp}/renoir_abi_target"
CARGO_PROFILE_RELEASE_DEBUG=2 CARGO_TARGET_DIR="$tgt" cargo build --release --manifest-path "$root/Cargo.toml" >/dev/null || exit 1
abidw --drop-private-types --no-show-locs --no-corpus-path --headers-dir "$root/include" \
  --out-file "$tgt/current.abi" "$tgt/release/librenoir.so" || exit 1
abidiff "$root/tests/abi/librenoir.abi" "$tgt/current.abi"
rc=$?
if [ $((rc & 1)) -ne 0 ] || [ $((rc & 2)) -ne 0 ]; then
  echo "abidiff failed to run (exit $rc)"
  exit 1
fi
if [ $((rc & 8)) -ne 0 ]; then
  echo "INCOMPATIBLE ABI change detected"
  exit 1
fi
if [ $((rc & 4)) -ne 0 ]; then
  echo "compatible ABI change: regenerate tests/abi/librenoir.abi with abidw"
fi
exit 0
