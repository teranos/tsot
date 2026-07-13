#!/usr/bin/env sh
# Populate a local CDDA source tree for non-Nix builds.
#
# CDDA is a *dependency*, not vendored source: this fetches it, git
# never stores it. Pinned to the stable letter release named in
# ./CDDA_RELEASE (e.g. 0.I). Sparse + shallow — only the mapgen +
# palette subtrees, one commit — so any building can be referenced
# without maintaining per-file copies.
#
# Output: game/.cdda-src/ (gitignored). build.rs reads from here when
# $CDDA_SRC is unset (the Nix flake sets $CDDA_SRC to the pinned input
# instead, so this script is only for bare-cargo / CI builds).
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)          # game/
rel=$(tr -d ' \t\r\n' < "$here/CDDA_RELEASE")
dst="$here/.cdda-src"
repo="https://github.com/CleverRaven/Cataclysm-DDA.git"

# The exact files to materialise — one manifest, shared with build.rs
# and game/flake.nix (no duplicated file list).
files=$(grep -vE '^\s*(#|$)' "$here/cdda-files.txt")

echo "[fetch-cdda] CDDA $rel -> .cdda-src (sparse: $(echo "$files" | wc -l | tr -d ' ') files)"
rm -rf "$dst"
git clone --depth 1 --filter=blob:none --sparse --branch "$rel" "$repo" "$dst" >/dev/null 2>&1
# Non-cone so we get exactly those files, not their whole directories.
# shellcheck disable=SC2086
git -C "$dst" sparse-checkout set --no-cone $files >/dev/null 2>&1
echo "[fetch-cdda] pinned commit $(git -C "$dst" rev-parse HEAD)"
