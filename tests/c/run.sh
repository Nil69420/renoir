#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
profile=${1:-debug}
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
cargo build --manifest-path "$root/Cargo.toml" $([ "$profile" = release ] && echo --release)
cc -std=c11 -Wall -Wextra -Wpedantic -Werror -fstack-protector-strong -fsanitize=undefined \
  -I"$root/include" "$root/tests/c/lean_abi_test.c" -L"$root/target/$profile" -lrenoir -o "$out/lean_abi"
LD_LIBRARY_PATH="$root/target/$profile" "$out/lean_abi"
LD_LIBRARY_PATH="$root/target/$profile" valgrind --leak-check=full --error-exitcode=1 -q "$out/lean_abi"
