#!/usr/bin/env sh
# Populate a local CDDA source tree for non-Nix builds.
#
# CDDA is a *dependency*, not vendored source: this fetches it, git
# never stores it. Pinned to the stable letter release named in
# ./CDDA_RELEASE (e.g. 0.I).
#
# Fetches the whole mapgen + palette SUBTREES (not the exact files in
# cdda-files.txt). This is deliberate: the fetched set — and therefore
# game/flake.nix's content hash — must NOT depend on cdda-files.txt.
# Adding a building is one line in cdda-files.txt; it must not change
# what's fetched or force a hash re-pin. build.rs picks the exact files
# to embed from these subtrees.
#
# Output: game/.cdda-src/ (gitignored). build.rs reads from here when
# $CDDA_SRC is unset (the Nix flake sets $CDDA_SRC to the pinned input
# instead, so this script is only for bare-cargo / CI builds).
#
# Keep these patterns identical to sparseCheckout in game/flake.nix, so
# the script and the flake produce the same tree.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)          # game/
rel=$(tr -d ' \t\r\n' < "$here/CDDA_RELEASE")
dst="$here/.cdda-src"
repo="https://github.com/CleverRaven/Cataclysm-DDA.git"

echo "[fetch-cdda] CDDA $rel -> .cdda-src (subtrees: mapgen + mapgen_palettes)"
rm -rf "$dst"
git clone --depth 1 --filter=blob:none --sparse --branch "$rel" "$repo" "$dst" >/dev/null 2>&1
git -C "$dst" sparse-checkout set --no-cone \
    'data/json/mapgen' 'data/json/mapgen_palettes' >/dev/null 2>&1
# Stamp the release so build.rs can detect a stale .cdda-src (bumping
# CDDA_RELEASE without a re-fetch would otherwise compile the old corpus
# silently).
printf '%s\n' "$rel" > "$dst/.rev"
echo "[fetch-cdda] pinned commit $(git -C "$dst" rev-parse HEAD)"
