#!/bin/sh
set -eu
cd "$(dirname "$0")"
JAR=${TLA2TOOLS:-$HOME/.local/tla/tla2tools.jar}
if [ ! -f "$JAR" ]; then
  mkdir -p "$(dirname "$JAR")"
  curl -sL -o "$JAR" https://github.com/tlaplus/tlaplus/releases/latest/download/tla2tools.jar
fi
tlc() { java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC -workers 4 -deadlock "$@" 2>&1; }
echo "OLD rule must violate both invariants (the model reproduces the bugs)"
tlc -config LeanMP_OLD.cfg LeanMP.tla | grep -q "Invariant NoLossOfLiveProducer is violated"
sed 's/INVARIANTS.*/INVARIANTS NoTornRead/' LeanMP_OLD.cfg > "${TMPDIR:-/tmp}/LeanMP_OLD_torn.cfg"
tlc -config "${TMPDIR:-/tmp}/LeanMP_OLD_torn.cfg" LeanMP.tla | grep -q "Invariant NoTornRead is violated"
echo "NEW rule must satisfy both invariants"
for cfg in "2 2 4" "3 2 5" "2 4 7" "3 3 6" "3 3 8" "3 4 8" "4 3 7"; do
  set -- $cfg
  sed "s/P = 2 MaxIdx = 4/P = $1 MaxIdx = $3/; s/N = 2/N = $2/" LeanMP_NEW.cfg > "${TMPDIR:-/tmp}/LeanMP_NEW_big.cfg"
  tlc -config "${TMPDIR:-/tmp}/LeanMP_NEW_big.cfg" LeanMP.tla | grep -q "No error has been found"
  echo "NEW ok: P=$1 N=$2 MaxIdx=$3"
done
echo "FrameGate: the timer rule must violate ownership, the death rule must not"
tlc -config FrameGate_Timer.cfg FrameGate.tla | grep -q "Invariant NoOverwriteWhileHeld is violated"
for cfg in "2 5 {c1,c2}" "3 6 {c1,c2}" "2 5 {c1,c2,c3}" "3 7 {c1,c2,c3}"; do
  set -- $cfg
  sed "s/N = 2 MaxIdx = 5 Consumers = {c1, c2}/N = $1 MaxIdx = $2 Consumers = $3/" FrameGate_Death.cfg > "${TMPDIR:-/tmp}/FrameGate_big.cfg"
  tlc -config "${TMPDIR:-/tmp}/FrameGate_big.cfg" FrameGate.tla | grep -q "No error has been found"
  echo "FrameGate ok: N=$1 MaxIdx=$2 Consumers=$3"
done
echo "StampedSeqlock: without the sequence re-check a reader mixes two messages, with it never"
tlc -config StampedSeqlock_NoRecheck.cfg StampedSeqlock.tla | grep -q "Invariant NoMixedMessage is violated"
for cfg in "2 8" "3 12" "4 14"; do
  set -- $cfg
  sed "s/N = 2 MaxIdx = 8/N = $1 MaxIdx = $2/" StampedSeqlock.cfg > "${TMPDIR:-/tmp}/StampedSeqlock_big.cfg"
  tlc -config "${TMPDIR:-/tmp}/StampedSeqlock_big.cfg" StampedSeqlock.tla | grep -q "No error has been found"
  echo "StampedSeqlock ok: N=$1 MaxIdx=$2"
done
echo "formal: all green"
