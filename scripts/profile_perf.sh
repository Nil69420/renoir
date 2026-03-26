#!/usr/bin/env bash
set -euo pipefail

# Run perf, flamegraph, and gdb profiling from the repository root.
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required" >&2
  exit 1
fi

echo "[1/5] Building release benchmark binaries"
cargo build --release --bench allocator_bench --bench ringbuf_bench

echo "[2/5] Running perf stat on CPU performance test"
if command -v perf >/dev/null 2>&1; then
  perf stat -e cache-misses,cache-references,instructions,cycles,branch-misses \
    cargo test --release --test cpu_performance_tests -- --nocapture || true
else
  echo "perf not found; skipping perf stat"
fi

echo "[3/5] Running flamegraph on allocator and ring buffer benches"
if command -v cargo-flamegraph >/dev/null 2>&1 || cargo flamegraph --help >/dev/null 2>&1; then
  cargo flamegraph --bench allocator_bench -o flamegraph_allocator.svg -- --bench || true
  cargo flamegraph --bench ringbuf_bench -o flamegraph_ringbuf.svg -- --bench || true
else
  echo "cargo flamegraph is not installed; skipping flamegraph generation"
fi

echo "[4/5] Capturing gdb backtrace snapshot from ring buffer benchmark"
if command -v gdb >/dev/null 2>&1; then
  BENCH_BIN="$(find target/release/deps -maxdepth 1 -type f -name 'ringbuf_bench-*' -executable | head -n1 || true)"
  if [[ -n "$BENCH_BIN" ]]; then
    timeout 90 gdb -q -batch \
      -ex "set pagination off" \
      -ex "run --bench" \
      -ex "thread apply all bt full" \
      --args "$BENCH_BIN" || true
  else
    echo "No ringbuf benchmark binary found; skipping gdb"
  fi
else
  echo "gdb not found; skipping gdb backtrace"
fi

echo "[5/5] Done"
echo "Artifacts (if generated): flamegraph_allocator.svg, flamegraph_ringbuf.svg"
