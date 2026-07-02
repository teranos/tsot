#!/usr/bin/env bash
# Content-hash and rename the three artifacts of one backend build
# with the backend suffix baked into the filename, so both backend
# builds coexist flat in dist/ without collision.
#
# Args:
#   $1 — dist directory (dist/)
#   $2 — backend suffix (webgpu or webgl2)
#
# Reads the transient filenames wasm-bindgen + bun produced this
# invocation (rave.js, rave_bg.wasm, main.js) and renames them to
# their per-backend hashed final forms. Only touches those three;
# other backend's already-renamed files are left alone.
set -euo pipefail

DIR="$(cd "$1" && pwd)"
BACKEND="$2"

cd "$DIR"

wh=$(sha256sum rave_bg.wasm | cut -c1-12)
mv rave_bg.wasm "rave-${BACKEND}-bg.${wh}.wasm"
sed -i "s|rave_bg\.wasm|rave-${BACKEND}-bg.${wh}.wasm|g" rave.js

jh=$(sha256sum rave.js | cut -c1-12)
mv rave.js "rave-${BACKEND}.${jh}.js"
sed -i "s|\./rave\.js|./rave-${BACKEND}.${jh}.js|g" main.js
sed -i "s|WASM_URL_PLACEHOLDER|./rave-${BACKEND}-bg.${wh}.wasm|g" main.js

mh=$(sha256sum main.js | cut -c1-12)
mv main.js "main-${BACKEND}.${mh}.js"
