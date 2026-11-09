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
cc -std=c11 -Wall -Wextra -Wpedantic -Werror -fstack-protector-strong -fsanitize=undefined -pthread \
  -I"$root/include" "$root/tests/c/rpc_abi_test.c" -L"$root/target/$profile" -lrenoir -o "$out/rpc_abi"
LD_LIBRARY_PATH="$root/target/$profile" "$out/rpc_abi"
LD_LIBRARY_PATH="$root/target/$profile" valgrind --leak-check=full --error-exitcode=1 -q "$out/rpc_abi"
cc -std=c11 -Wall -Wextra -Wpedantic -Werror -fstack-protector-strong -fsanitize=undefined \
  -I"$root/include" "$root/tests/c/frame_abi_test.c" -L"$root/target/$profile" -lrenoir -o "$out/frame_abi"
LD_LIBRARY_PATH="$root/target/$profile" "$out/frame_abi"
LD_LIBRARY_PATH="$root/target/$profile" valgrind --leak-check=full --error-exitcode=1 -q "$out/frame_abi"
cc -std=c11 -Wall -Wextra -Wpedantic -Werror -fstack-protector-strong -fsanitize=undefined \
  -I"$root/include" "$root/tests/c/discovery_watch_abi_test.c" -L"$root/target/$profile" -lrenoir -o "$out/discovery_watch_abi"
LD_LIBRARY_PATH="$root/target/$profile" "$out/discovery_watch_abi"
LD_LIBRARY_PATH="$root/target/$profile" valgrind --leak-check=full --error-exitcode=1 -q "$out/discovery_watch_abi"
