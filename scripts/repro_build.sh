#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
epoch="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct)}"
build() {
  local dir="$1"
  git -C "$root" worktree add --detach "$dir" HEAD >/dev/null
  (
    cd "$dir"
    export SOURCE_DATE_EPOCH="$epoch"
    export CARGO_TARGET_DIR="$dir/target"
    export RUSTFLAGS="--remap-path-prefix=$dir=/build --remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$HOME/.rustup=/rustup"
    cargo build --release --locked --lib
  )
  sha256sum "$dir/target/release/librenoir.so" | awk '{print $1}'
  git -C "$root" worktree remove --force "$dir"
}
first="$(build "$work/a")"
second="$(build "$work/b")"
echo "build A: $first"
echo "build B: $second"
if [ "$first" = "$second" ]; then
  echo "REPRODUCIBLE"
else
  echo "NOT REPRODUCIBLE"
  exit 1
fi
