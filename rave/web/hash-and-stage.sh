#!/usr/bin/env bash
# Content-hash the three artifacts in a per-backend build subdir and
# emit `index.html` from the shared template.
#
# Args:
#   $1 — build subdir (dist/webgpu or dist/webgl2)
#   $2 — html template (web/index.html)
#
# Called twice from the Makefile — once per backend. Extracted into
# its own script so the hashing pass stays a single source of truth.
set -euo pipefail

DIR="$(cd "$1" && pwd)"
TEMPLATE="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"

cd "$DIR"

wh=$(sha256sum rave_bg.wasm | cut -c1-12)
mv rave_bg.wasm "rave_bg.$wh.wasm"
sed -i "s/rave_bg\.wasm/rave_bg.$wh.wasm/g" rave.js

jh=$(sha256sum rave.js | cut -c1-12)
mv rave.js "rave.$jh.js"
sed -i "s|\./rave\.js|./rave.$jh.js|g" main.js
sed -i "s|WASM_URL_PLACEHOLDER|./rave_bg.$wh.wasm|g" main.js

mh=$(sha256sum main.js | cut -c1-12)
mv main.js "main.$mh.js"
sed -e "s|\./main\.js|./main.$mh.js|g" "$TEMPLATE" > index.html
