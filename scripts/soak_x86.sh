#!/bin/sh
set -u
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
SECONDS_TOTAL=${1:-480}
SUITES="--test lean_chaos_tests --test lean_chaos_multi_tests --test lean_chaos_gate_tests --test lean_stall_tests --test lean_frame_chaos_tests --test rpc_shard_tests --test lean_gate_tests --test lean_deps_tests --test lean_frame_tests --test region_size_safety_tests --test discovery_tests"
cargo test --profile testing $SUITES --no-run >/dev/null 2>&1 || { echo "build failed"; exit 2; }
end=$(( $(date +%s) + SECONDS_TOTAL ))
runs=0
fails=0
while [ "$(date +%s)" -lt "$end" ]; do
  runs=$((runs + 1))
  if ! out=$(cargo test --profile testing $SUITES 2>&1); then
    fails=$((fails + 1))
    echo "== failure in run $runs"
    echo "$out" | grep -E "FAILED|panicked|test result: F" | head -5
  fi
done
echo "soak_x86: $runs runs, $fails failed, $SECONDS_TOTAL s"
[ "$fails" -eq 0 ]
