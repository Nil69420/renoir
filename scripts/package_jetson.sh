#!/bin/bash
# Cross-builds Renoir for aarch64 (Jetson, Ubuntu 22.04 / JetPack 6 glibc 2.35)
# and packs the library, headers, CLI and soak test into dist/.
set -eu
cd "$(dirname "$0")/.."
TARGET=aarch64-unknown-linux-gnu
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
rustup target add "$TARGET"
cargo build --release --target "$TARGET" --features "c-api cli"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
NAME=renoir-jetson-$VERSION-aarch64
OUT=dist/$NAME
rm -rf "$OUT"
mkdir -p "$OUT/lib" "$OUT/include" "$OUT/bin"
cp target/$TARGET/release/librenoir.so "$OUT/lib/librenoir.so.0"
ln -s librenoir.so.0 "$OUT/lib/librenoir.so"
cp include/renoir.h include/renoir.hpp include/renoir_lean.hpp include/renoir_rpc.hpp include/renoir_discovery.hpp include/renoir_frame.hpp "$OUT/include/"
cp target/$TARGET/release/renoir-cli "$OUT/bin/"
cp scripts/jetson_soak.sh tools/soak/renoir_soak.c "$OUT/"
cat > "$OUT/README.txt" <<'EOT'
Renoir for Jetson (aarch64). Built against glibc 2.35 (Ubuntu 22.04 / JetPack 6).

  lib/librenoir.so.0    shared library (C ABI, SONAME librenoir.so.0)
  include/renoir.h      C header     include/renoir.hpp  C++ wrapper
  bin/renoir-cli        introspection: ./bin/renoir-cli topics [--watch]
  jetson_soak.sh        soak test (needs gcc on the device)

Quick check:   ./jetson_soak.sh 30            (30 s, 2 cameras, 120 fps, 1 kHz IMU)
Full soak:     ./jetson_soak.sh 600 2 120 1000
Rings live in /dev/shm (RAM). Override with RENOIR_SHM_DIR=/some/tmpfs.
Run:  sudo nvpmodel -m 0 && sudo jetson_clocks   first for max performance runs.
EOT
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
sha256sum "dist/$NAME.tar.gz"
ls -la "dist/$NAME.tar.gz"
