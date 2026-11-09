#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/dist}"
mkdir -p "$out"
command -v cargo-cyclonedx >/dev/null || cargo install --locked cargo-cyclonedx
cd "$root"
cargo cyclonedx --format json --override-filename renoir.cdx
mv renoir.cdx.json "$out/renoir.cdx.json" 2>/dev/null || mv renoir.cdx "$out/renoir.cdx.json"
sha256sum "$out/renoir.cdx.json"
