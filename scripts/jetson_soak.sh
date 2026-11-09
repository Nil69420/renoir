#!/bin/bash
# Usage: jetson_soak.sh [duration_s=60] [cams=2] [fps=120] [imu_hz=1000]
# Runs one producer and three consumers (full-rate, VIO-like skip-to-latest,
# deliberately slow) against Renoir through the C ABI and prints PASS/FAIL.
set -u
DURATION=${1:-60}
CAMS=${2:-2}
FPS=${3:-120}
IMU=${4:-1000}

HERE=$(cd "$(dirname "$0")" && pwd)
LIB=${RENOIR_LIB_DIR:-$HERE/lib}
[ -d "$LIB" ] || LIB=$HERE/../target/release
INC=${RENOIR_INCLUDE_DIR:-$HERE/include}
[ -d "$INC" ] || INC=$HERE/../include
SRC=$HERE/renoir_soak.c
[ -f "$SRC" ] || SRC=$HERE/../tools/soak/renoir_soak.c
WORK=${SOAK_WORKDIR:-$(mktemp -d)}
BIN=$WORK/renoir_soak
CLI=$HERE/bin/renoir-cli
[ -x "$CLI" ] || CLI=$HERE/../target/release/renoir-cli

echo "== system"
uname -srm
echo "cpus: $(nproc)  governor: $(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo n/a)"
command -v nvpmodel >/dev/null 2>&1 && (nvpmodel -q 2>/dev/null | head -2)
df -h /dev/shm | tail -1
echo "shm dir: ${RENOIR_SHM_DIR:-/dev/shm (default)}"

cc -std=c11 -O2 -Wall -Wextra -pthread "$SRC" -I"$INC" -L"$LIB" -lrenoir \
   -Wl,-rpath,"$LIB" -o "$BIN" || { echo "build failed"; exit 2; }
export LD_LIBRARY_PATH="$LIB${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

ARGS="--cams $CAMS --fps $FPS --imu-hz $IMU --duration $DURATION --start-delay 4"
echo "== run: $ARGS"

TEG=""
if command -v tegrastats >/dev/null 2>&1; then
  timeout $((DURATION + 8)) tegrastats --interval 1000 > "$WORK/tegrastats.log" 2>&1 &
  TEG=$!
fi

"$BIN" consumer $ARGS --label full        > "$WORK/full.out" 2>&1 &
P1=$!
"$BIN" consumer $ARGS --latest 1 --slow-ms 15 --label vio_like > "$WORK/vio.out" 2>&1 &
P2=$!
"$BIN" consumer $ARGS --slow-ms 50 --label slow > "$WORK/slow.out" 2>&1 &
P3=$!
sleep 1
"$BIN" producer $ARGS --label producer > "$WORK/producer.out" 2>&1 &
PP=$!

sleep $((4 + DURATION / 2))
if [ -x "$CLI" ]; then echo "== renoir-cli topics (mid-run)"; "$CLI" topics 2>&1 | head -8; fi

FAIL=0
for p in $PP $P1 $P2 $P3; do wait $p || FAIL=1; done
[ -n "$TEG" ] && kill $TEG 2>/dev/null

for f in producer full vio slow; do echo; cat "$WORK/$f.out"; done
if [ -f "$WORK/tegrastats.log" ]; then
  echo; echo "== tegrastats (last 3 lines, full log: $WORK/tegrastats.log)"
  tail -3 "$WORK/tegrastats.log"
fi
echo
if [ $FAIL -eq 0 ]; then echo "OVERALL: PASS"; else echo "OVERALL: FAIL (see $WORK)"; fi
exit $FAIL
