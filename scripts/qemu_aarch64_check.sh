#!/bin/sh
set -eu
TARGET=aarch64-unknown-linux-gnu
export QEMU_LD_PREFIX=/usr/aarch64-linux-gnu
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64-static -L /usr/aarch64-linux-gnu"
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
rustup target add "$TARGET"
cargo build --release --target "$TARGET" --features c-api
cargo test --release --target "$TARGET" --features c-api \
    --test broadcast_ring_tests --test broadcast_topic_tests \
    --test cross_process_tests --test ffi_basic_tests \
    --test lean_ring_tests --test lean_zero_alloc
