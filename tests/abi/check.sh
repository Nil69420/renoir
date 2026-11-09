#!/bin/sh
set -eu
export LC_ALL=C
root=$(cd "$(dirname "$0")/../.." && pwd)
cargo build --release --manifest-path "$root/Cargo.toml"
nm -D --defined-only "$root/target/release/librenoir.so" | awk '$2=="T"{print $3}' | grep '^renoir_' | sort > "${TMPDIR:-/tmp}/renoir_symbols_now.txt"
sort "$root/tests/abi/symbols.txt" > "${TMPDIR:-/tmp}/renoir_symbols_base.txt"
missing=$(comm -23 "${TMPDIR:-/tmp}/renoir_symbols_base.txt" "${TMPDIR:-/tmp}/renoir_symbols_now.txt")
if [ -n "$missing" ]; then
  echo "ABI break: exported symbols removed or renamed:"
  echo "$missing"
  exit 1
fi
added=$(comm -13 "${TMPDIR:-/tmp}/renoir_symbols_base.txt" "${TMPDIR:-/tmp}/renoir_symbols_now.txt")
if [ -n "$added" ]; then
  echo "New exported symbols (update tests/abi/symbols.txt):"
  echo "$added"
  exit 2
fi
echo "ABI symbols match baseline"
