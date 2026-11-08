#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
status=0
for f in *.litmus; do
  case "$f" in
    *_CONTROL.litmus) want="Sometimes" ;;
    *) want="Never" ;;
  esac
  got=$(herd7 -model aarch64.cat "$f" | awk '/^Observation/ {print $3}')
  if [ "$got" = "$want" ]; then
    echo "ok   $f ($got)"
  else
    echo "FAIL $f: expected $want, got $got"
    status=1
  fi
done
exit $status
