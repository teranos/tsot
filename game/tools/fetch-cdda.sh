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
expected_commit=$(tr -d ' \t\r\n' < "$here/CDDA_COMMIT")
dst="$here/.cdda-src"
repo="https://github.com/CleverRaven/Cataclysm-DDA.git"

echo "[fetch-cdda] CDDA $rel -> .cdda-src (subtrees: mapgen + mapgen_palettes)"
rm -rf "$dst"
git clone --depth 1 --filter=blob:none --sparse --branch "$rel" "$repo" "$dst" >/dev/null 2>&1
git -C "$dst" sparse-checkout set --no-cone \
    'data/json/mapgen' 'data/json/mapgen_palettes' >/dev/null 2>&1
# Integrity check: verify the fetched HEAD matches CDDA_COMMIT. The nix
# path is hash-pinned by fetchFromGitHub; this script path had no
# verification of what it downloaded — a moved tag or a mirror-in-the-
# middle could silently swap out the corpus. Same pin here.
actual_commit=$(git -C "$dst" rev-parse HEAD)
if [ "$actual_commit" != "$expected_commit" ]; then
    echo "[fetch-cdda] FAIL: expected commit $expected_commit but got $actual_commit" >&2
    echo "[fetch-cdda] If CleverRaven moved the $rel tag, update CDDA_COMMIT AND flake.nix's hash together." >&2
    exit 1
fi
# Stamp the release so build.rs can detect a stale .cdda-src (bumping
# CDDA_RELEASE without a re-fetch would otherwise compile the old corpus
# silently).
printf '%s\n' "$rel" > "$dst/.rev"
echo "[fetch-cdda] pinned commit $actual_commit (verified)"
