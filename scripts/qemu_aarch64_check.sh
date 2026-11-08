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
    --test lean_ring_tests --test lean_zero_alloc --test zero_alloc_paths \
    --test lean_wait_tests --test lean_multi_tests --test lean_gate_tests \
    --test lean_batch_tests --test lean_qos_tests --test rpc_tests \
    --test broadcast_view_tests --test ring_init_race_tests --test lean_shuttle \
    --test ffi_view_tests
