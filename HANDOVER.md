# HANDOVER — branch `claude/lavapipe-graphics-8qk7zx`

For the next session reviewing this branch. This is a session-to-session
note, not a design doc — it does **not** restate the design. The design,
the slices, what shipped, and the deferred list all live in
[`game/docs/TERRAIN.md`](game/docs/TERRAIN.md). Read that first; this
covers only what a reviewer/continuer needs that isn't in there.

## What this branch is

SimCity 4-style terrain height for `game/`, built and validated on
headless lavapipe (software Vulkan → PNG) and confirmed live at
game.sbvh.nl. All 8 slices + grid redesign + collision fix landed. State
of each slice: see the TERRAIN.md Status section and checklist.

## Branch state

- **Tip:** `994a78d` — 21 commits ahead of `origin/master`.
- **Scope of the diff:** `game/` only (11 files, ~+1400/-95). Diffstat:
  `git diff --stat origin/master...HEAD`.
- CI was last green at `28e9459` (the last code commit;
  `d7e6b53`/`ffddf03`/`994a78d` after it are the XZ-collision fix + doc
  only). No PR opened — the user wants this session's work reviewed by a
  fresh session before any PR.
- Nothing uncommitted; working tree clean.

## Build / test / run (traps that will bite you)

- **Nightly is mandatory.** `cargo +nightly …` everywhere — bevy_ecs
  0.19 needs rustc ≥ 1.95. A plain `cargo` invocation will fail to
  compile; that's the toolchain, not the code.
- **Native tests:** `cd game && cargo +nightly test`.
- **Headless render (the proof channel):** the `game-native` binary
  under lavapipe. Force the software ICD:
  `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`. Frames driven
  by `SEER_MULTI_FRAME_DIR` / `SEER_FRAMES` → 512×512 PNGs; the tour
  visits school / house / orchard / campsite / forest. `make` targets
  exist in `game/` for the daily-driver path.
- **Perf number:** `cargo +nightly run --release --example terrain_perf`
  (≈0.12 ms/frame terrain geometry). Don't quote perf from intuition —
  seer's `[perf]` / this example are the only sources.

## CI gates — must be green before a PR

Workflows fire on push to `**`. The three that matter here:

- **`game-tests.yml`** — native `cargo +nightly test` **and**
  `clippy -D warnings`. This one caught us before: clippy is a hard gate.
- **`seer.yml`** — wasm build + the no-wasm-bindgen boundary check
  (`seer-imports-check`) + seer-host render.
- **`deploy-game.yml`** — wasm release build + bun bundle → game.sbvh.nl.

**Before pushing any code change, gate locally on clippy for BOTH
targets** — this is the failure mode that has repeatedly gone red:
```
cargo +nightly clippy --lib -- -D warnings
cargo +nightly clippy --lib --target wasm32-unknown-unknown -- -D warnings
```

## The no-wasm-bindgen boundary (do not break)

Every wasm↔browser call is a hand-wired `env.*` import declared in
`game/imports.allow`, enforced by `crates/seer-imports-check`. This
branch added **zero** new crossings — the browser terrain reused the
existing mesh shader/pipeline/render crossings. If you extend the render,
reuse existing crossings or the boundary check fails; `imports.allow`
should not need to change for terrain work.

## Key files (orientation, not a spec)

- `game/src/terrain.rs` — `height(x,z)`: base value-noise + CDDA pad
  flatten + skirt. The single source of truth.
- `game/src/scene.rs` — surface mesh, `surface_snap` (regen cache key),
  `drape`/`drape_mesh` (the whole-scene drape choke point).
- `game/src/shaders.rs` — `GROUND_SHADER_WGSL` (the world-anchored grid
  is fragment math here, not geometry).
- `game/src/render.rs` / `render_web.rs` — native and browser paths;
  keep them in parity.
- `game/src/physics.rs` — `ground_follow_*` (sim height), XZ
  `resolve_collisions`.

## Open items the reviewer should weigh

These are surfaced in TERRAIN.md's follow-ups; flagged here as the things
a reviewer is most likely to question:

1. **Browser merge bar is the deployed site, not a captured frame.** The
   automated headless-Chromium screenshot stayed flaky (async GPU-device
   init stalls under `--virtual-time-budget`; CDP flaky in-sandbox). The
   user verified game.sbvh.nl live instead. If you want a captured
   headless frame, that's the unsolved capture problem — not a terrain
   bug.
2. **Collision is XZ-only** (ground plane). Deliberate: with real sim
   height, 3D colliders authored at `y` never overlap a player on a hill
   ("Casper" bug). Proximity triggers (NPC bump, jukebox) are still
   3D-distance — harmless while relief is gentle, XZ-able if they misfire.
3. **House-rules to honour** (repo CLAUDE.md): strict TDD (failing test
   first), errors surfaced never swallowed, and — importantly for a
   reviewer — **the render PNG is the only accepted proof**; accept the
   user's testimony about what a render shows without re-litigating.

## If the PR for this branch is already merged when you pick this up

Per the repo's branch rule: a merged PR is finished. Restart this branch
name from `origin/master` for any follow-up (`git fetch origin master &&
git checkout -B claude/lavapipe-graphics-8qk7zx origin/master`); don't
stack new work on merged history.
