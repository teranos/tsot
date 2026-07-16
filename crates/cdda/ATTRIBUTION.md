# CDDA map data — attribution

The game's buildings are assembled from **Cataclysm: Dark Days Ahead**
map data, by CleverRaven and contributors.

- Source: https://github.com/CleverRaven/Cataclysm-DDA
- License: **Creative Commons Attribution-ShareAlike 3.0 (CC-BY-SA 3.0)**
  — https://creativecommons.org/licenses/by-sa/3.0/
- Pinned release: the stable **letter** release named in
  [`../../CDDA_RELEASE`](../../CDDA_RELEASE) (currently `0.I`).

**Not vendored.** No CDDA JSON lives in this repository. The corpus is a
build-time dependency, fetched from the pinned release and materialised
into the build (`build.rs`) — never committed:

- **Nix:** the `cataclysm-dda` fetch in `game/flake.nix` pins the
  release by content hash (verified on every build; locked provenance).
- **Bare cargo / CI:** `game/tools/fetch-cdda.sh` sparse-clones the same
  release into `.cdda-src/` (gitignored).

Which mapgen + palette files are pulled in is listed in `build.rs`
(`CDDA_FILES`) — references into the pinned corpus, not copies. Adding a
building adds a reference there; it never adds vendored bytes.

Any building rendered in game is a derivative of this data and remains
under CC-BY-SA 3.0, attributed to CleverRaven / the Cataclysm: Dark Days
Ahead project.

(The shed under `assets/buildings/` is an original layout, ours, not
CDDA content.)
