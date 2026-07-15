# game — handover

What's not in PR #16. The delivered work is described in the PR body;
the watermark top-right in-game is ground truth for what's running.

## Open — not in the PR

- **Drive the un-verified flows on device.** Music/SFX persist across
  reload, both sliders, ESC/gear close, jukebox toggle visible,
  ghost + cut-away between two adjacent buildings.
- **Pre-merge:** squash the vendoring churn + scrub the CC-BY-SA JSON
  blobs from history before merging.

## The frontier (open research)

1. **Can the world hold the whole corpus?** `place_nested`, vehicles,
   monsters, multi-tile specials — which unhandled features must land
   to grow past lone buildings? (Our `shed.json` exists only because
   CDDA has no standalone shed; when nested mapgen lands, it can go.)
2. **When does a building stop being scenery?** Which CDDA flags become
   behaviour — TRANSPARENT→glass, doors open/close,
   CONTAINER/SEALED→loot? The hand-placed jukebox is a CDDA furniture
   entry we could *read* instead of *author*.
3. **Should the generator BE CDDA's grammar?** `palette.rs` already
   rolls per-building seeds through variant palettes. How far up
   (block, town) does authored parameter/distribution reach as the
   world generator vs our per-chunk hash?
4. **Can you go down and up?** Roof cut-away + ghost pass is the first
   z-level UX. CDDA ships basements + upper floors as their own
   mapgen. What's descend/ascend in an iso voxel world?
5. **Do lone buildings become towns?** CDDA authors roads, blocks,
   connected specials. Settlement coherence — more authored data
   through the same pipeline, or new placement logic?
6. **Does the map feed combat?** roam v0.5 invokes the ccg engine for
   PvP; CDDA mapgen carries monster spawns. A building's authored
   spawns → encounters resolved by ccg — map as encounter source, not
   just architecture.

## Not a defect — leave it alone

- **Glass** looks correct — user-verified on device. The glass pass
  draws panes unsorted with depth-write off (theoretically
  order-dependent). In practice it's fine. Only if you ever see wrong
  blending where many panes overlap, add a back-to-front sort in
  `scene::snapshot_to_glass_instances`. Do not pre-emptively "fix" it.

## Environment gotchas

- **No GPU in the dev sandbox** — native wgpu render + any visual
  check fails (`No suitable graphics adapter`). Only the user's device
  confirms visuals. "Tests pass" is necessary, not sufficient.
- **`api.github.com` blocked** — nix flake *input* resolution via
  `github:` fails; the git mirror works (`builtins.fetchGit`, plain
  `git clone` succeed). CDDA hash verified via `builtins.fetchGit` +
  `nix hash path`, not full `nix build`; `flake.lock` couldn't be
  generated in the sandbox.
- **`imports.allow` is enforced** — every wasm↔JS crossing is a
  hand-wired `env.*` import; adding one edits `imports.allow` +
  `main.ts` + the Rust extern, and CI diffs it.

## Lessons — don't repeat these

- **Counts are not performance.** `game/CLAUDE.md` says "Observed by
  seer" — no perf claim without a seer `[perf]` line. Standing in a
  3,755-prop school costs ~0.4ms/frame steady-state; the only real
  cost is a one-frame load hitch.
- **A green pipeline is not a live measurement.** For 9 commits the
  deploy was broken because "live == HEAD" was asserted once and never
  re-checked; the flake hash was a function of the file list, so
  adding a building silently broke `nix build .#cdda-src` in CI while
  local cargo (which uses the `.cdda-src` fetch path — no hash check)
  stayed green. Fixed by decoupling the fetch from the manifest;
  verify `build-info.json` == HEAD after any deploy-affecting push.
- **A swallowed error rots for days.** seer looked alive (green jobs,
  frame PNGs uploading) but was measuring **nothing** for the whole
  branch because 11 wasm imports weren't mirrored into seer-host's
  linker and the run step piped through `tee` (masking the crash).
  Errors are sacred — read the artifact, not the job colour.
