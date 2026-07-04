#!/usr/bin/env bash
# Build the POC to a loadable web bundle in ./dist.
#
# Requires: the wasm32-unknown-unknown target and wasm-bindgen-cli
# whose version MATCHES the wasm-bindgen crate pinned in Cargo.toml
# (=0.2.121). Mismatched versions produce a runtime schema error.
#
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.121
#
# Then, from rave-wgpu-poc/:
#   ./web/build.sh            # release build
#   python3 -m http.server -d dist 8080   # serve, open on the phone
set -euo pipefail

cd "$(dirname "$0")/.."

PROFILE="${1:-release}"
OUT=dist
mkdir -p "$OUT"

echo "→ cargo build ($PROFILE, wasm32)…"
if [ "$PROFILE" = "release" ]; then
  cargo build --release --target wasm32-unknown-unknown
  WASM=target/wasm32-unknown-unknown/release/rave_wgpu_poc.wasm
else
  cargo build --target wasm32-unknown-unknown
  WASM=target/wasm32-unknown-unknown/debug/rave_wgpu_poc.wasm
fi

echo "→ wasm-bindgen → $OUT/…"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" "$WASM"

# Optional size pass — skipped if wasm-opt is absent.
if command -v wasm-opt >/dev/null 2>&1; then
  echo "→ wasm-opt -Oz…"
  wasm-opt -Oz -o "$OUT/rave_wgpu_poc_bg.wasm" "$OUT/rave_wgpu_poc_bg.wasm"
fi

cp web/index.html "$OUT/index.html"
echo "✓ built $OUT/  ($(du -h "$OUT"/rave_wgpu_poc_bg.wasm | cut -f1) wasm)"
echo "  serve: python3 -m http.server -d $OUT 8080"
