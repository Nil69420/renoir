#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
ADDONS=${MISRA_ADDON_DIR:-${TMPDIR:-/tmp}/cppcheck_addons}
TAG=${CPPCHECK_TAG:-2.7}
mkdir -p "$ADDONS"
for f in misra.py cppcheckdata.py misra_9.py; do
  [ -f "$ADDONS/$f" ] || curl -sfL -o "$ADDONS/$f" "https://raw.githubusercontent.com/danmar/cppcheck/$TAG/addons/$f"
done
printf '{"script":"%s/misra.py","args":[]}\n' "$ADDONS" > "$ADDONS/misra.json"
FILES="tests/c/lean_abi_test.c tests/c/rpc_abi_test.c tests/c/frame_abi_test.c tests/c/lean_stamped_abi_test.c tests/c/discovery_watch_abi_test.c tools/soak/renoir_soak.c"
echo "== cppcheck MISRA C:2012 addon (rule numbers only, proprietary texts not used)"
cppcheck --addon="$ADDONS/misra.json" --enable=style --suppress=missingIncludeSystem -Iinclude $FILES 2>&1 \
  | grep -E "misra" | sed 's/.*\[\(.*\)\]$/\1/' | sort | uniq -c | sort -rn
echo "== clang-tidy cert-*, bugprone-*, clang-analyzer-*"
clang-tidy $FILES -checks='-*,cert-*,bugprone-*,clang-analyzer-*' -- -Iinclude -std=c11 2>&1 \
  | grep -E "warning:" | sed 's/.*\[\(.*\)\]$/\1/' | sort | uniq -c | sort -rn
echo "== shipped header include/renoir.h (cppcheck MISRA, rule numbers)"
cppcheck --addon="$ADDONS/misra.json" --enable=style --suppress=missingIncludeSystem -Iinclude include/renoir.h 2>&1 \
  | grep -E "misra" | sed 's/.*\[\(.*\)\]$/\1/' | sort | uniq -c | sort -rn
