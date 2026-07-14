# game — handover

Working notes for continuing this branch. Blunt on purpose. A snapshot,
so re-verify live-vs-HEAD and re-read the checklist before trusting it.

Branch: **`stamp-template`**, pushed to `origin`. Live at game.sbvh.nl
(check `curl -s https://game.sbvh.nl/build-info.json` vs
`git rev-parse --short HEAD` — they should match; the in-game watermark
top-right is the ground truth). Snapshot taken at tip `1ce8439`.

## North star — where this is going

A world where CDDA's authored places are actually *there*: many
buildings, that **enclose** you, that you go **inside and upstairs and
down**, that carry **function not just shape**, arranged into **towns**,
**deterministic and identical for every peer**, with the map able to
**spawn the fights**. Everything the player sees is drawn by the Rust
engine; the running binary always tells the truth about itself.

The most interesting thing on this branch is the seam that serves that:
**CDDA mapgen is authored, human, canonical 2D tile data; our world is a
streaming, isometric, voxel, deterministic runtime. Cohering them is
projecting one medium's canon into another's engine.** Everything below
is either sharpening that projection or the hygiene that lets it scale.

## Do next — checklist

Ranked. The corpus is the point; the first four are what make growing it
safe, so do them roughly in order.

- [ ] **Coverage report over the full 0.I mapgen tree.** The first real
  corpus move and the one that makes the rest legible. For every
  `om_terrain` building, dry-run it through the resolver
  (`cdda::placement` / `cdda::cells::cell_to_prop`) and tally which
  symbols + palette parameters resolve vs silently drop to `None`. Flag
  `place_nested` / `place_vehicles` / `place_monsters` / multi-tile
  specials (all currently unhandled). Output: a ranked "cheapest
  buildings to add" list. Turns "add buildings" into a measured push.
- [ ] **`build.rs` basename-collision guard.** It copies each
  `cdda-files.txt` entry to `OUT_DIR/cdda/<basename>` with no uniqueness
  check; two same-basename files silently clobber and `include_str!`
  embeds the wrong bytes. Assert uniqueness (or copy under the full
  relative path) *before* the corpus grows and trips it.
- [ ] **Determinism golden-master.** Pin resolved building bytes per
  `(building, seed)` in a test. Placement is deterministic and tested,
  but resolved *output* isn't byte-pinned — and the projection just got
  branchier (flood-fill, edge shifts). This catches cross-peer drift
  before it ships as a desync. Prereq for more buildings AND P2P.
- [ ] **Commit `game/flake.lock`.** Root and `roam/` commit one; game
  doesn't, so `nix build .#cdda-src` floats nixpkgs/rust-overlay each
  run (corpus bytes stay hash-pinned; the *toolchain* doesn't).
  `cd game && nix flake lock` — needs real github access (the sandbox
  blocks `api.github.com` for flake inputs).
- [ ] **`.cdda-src` staleness guard.** `build.rs` uses the gitignored
  `.cdda-src` when `$CDDA_SRC` is unset, but nothing ties it to
  `CDDA_RELEASE`. Bump the release, forget `make cdda`, compile the old
  one silently. Write the fetched rev to `.cdda-src/.rev`; fail if ≠
  `CDDA_RELEASE`.
- [ ] **`tools/fetch-cdda.sh` integrity.** The nix path is hash-pinned;
  the script path compiles whatever the `0.I` tag returns, unverified.
  Checksum after fetch, or drop the script path and require nix.
- [ ] **Decide the JS build-match guard.** `main.ts` halts if the wasm's
  commit ≠ the bundle's. With the watermark + `?v=` cache-bust it's a
  narrow backstop. Keep it as a deliberate hard-stop or delete it — and
  confirm CloudFront's cache-key includes the query string, else `?v=`
  busts only the browser (the deploy invalidation is the real CDN
  refresh).
- [ ] **Unify `Music` resource access.** `hud` takes `ResMut<Music>`
  (panics if absent); `jukebox` takes `Option<ResMut<Music>>` (skips).
  Pick one — `Option<ResMut>` everywhere.
- [ ] **Drive the un-verified flows on device** (see Verification).
- [ ] **Open a PR.** Squash the vendoring churn and scrub the CC-BY-SA
  JSON blobs from history first.

## The big questions (the frontier)

Each is a want above, framed as the open question, with where to start.

1. **How much of the corpus can the world actually hold, and what's the
   honest ceiling?** The coverage report answers half; the other half is
   which unhandled features (`place_nested`, vehicles, monsters,
   multi-tile overmap specials) you must implement to break past a lone
   building. (Our own `shed.json` exists only because CDDA has no
   standalone shed — its sheds are `place_nested` pieces. When nested
   mapgen lands, the shed can go.)
2. **When does a building stop being scenery?** Which CDDA flags become
   behavior — `TRANSPARENT`→glass, doors open/close, `CONTAINER`/`SEALED`
   →loot? The hand-placed purple jukebox is exactly a CDDA furniture
   entry we could be *reading* instead of authoring. Cohering meaning,
   not just geometry.
3. **Should the generator *be* CDDA's grammar?** `palette.rs` already
   rolls a per-building seed through CDDA's variant palettes (we
   flattened its weights for visible variety). How far up — block, town —
   does the authored parameter/distribution + overmap layer reach as the
   world generator, vs our per-chunk hash?
4. **Can you go down and up?** Roof cut-away + the ghost pass is the
   first z-level UX. CDDA ships basements and upper floors as their own
   mapgen. What's descend/ascend in an iso voxel world?
5. **Do lone buildings become towns?** CDDA authors roads, blocks,
   connected specials. Is settlement coherence mostly more authored data
   through the same pipeline, or new placement logic?
6. **Does the map feed combat?** roam v0.5 invokes the ccg engine for PvP
   (repo `CLAUDE.md`); CDDA mapgen carries monster spawns. Could a
   building's authored spawns become encounters resolved by ccg — the map
   as an encounter source, not just architecture?

## Not a defect — leave it alone

- **Glass looks correct — user-verified on device.** The glass pass draws
  panes unsorted with depth-write off (theoretically order-dependent). In
  practice it's fine. Only if you ever see wrong blending where many
  panes overlap, add a back-to-front sort in
  `scene::snapshot_to_glass_instances`. Do not pre-emptively "fix" it.

## State now (post-pull)

- **Render passes (wasm):** opaque → **glass** (translucent windows) →
  **ghost** (cut-away walls/roof at alpha 0.15, so you see the outline of
  what you're inside instead of it vanishing) → **UI**. Ghost is the
  exact inverse filter of the opaque cut-away — emits only what opaque
  drops, no double-draw (`scene::snapshot_to_ghost_instances`).
- **CDDA → geometry:** module is now a directory
  `cdda/{parse,cells,placement,building,chunks}`. Walls are **edge-placed**
  — flood-fill from the mapgen boundary marks exterior, walls sit on the
  interior-facing grid line, corners emit L-segments to the inner corner,
  room dividers shift +z/+x ("always positive"), doors/gates block the
  flood so interiors don't leak. T-junction pillar protrusion fixed.
  Cut-away is anchored to the **overhead roof cell** (radius 800) so a
  neighbouring building stays solid.
- **HUD (all Rust UI quads):** music toggle (bottom-left), settings gear
  (top-left, tap to open/close), settings panel with **music + SFX**
  sliders; ESC also closes (desktop only — touch closes via the gear).
  The "!" bump indicator is now `bang.rs` quads (not DOM); NPC bumps play
  an MGS-style four-blip alert.
- **Persistence:** player position, music mix (mute + volume), SFX level
  — all localStorage via hand-wired env imports.
- **Version:** the in-Rust watermark (top-right) is the *sole* version
  indicator; the loading-screen build badge and `build-info.json` fetch
  are gone. `ui.rs` deleted; `?proxy=` escape hatch removed.
- **Boundary:** 44 hand-wired `env.*` imports. **Tests:** 88 lib green,
  clippy silent.
- **Corpus:** **7** CDDA buildings — garage, houses 01–04 (each ×6
  palette variants), the daycare, and the **school** (the first
  multi-tile building: a 3×3 / 72×72 grid) + our shed = 28 templates.
- **Multi-tile buildings work** (`chunk.rs`): a building's props are
  distributed to the chunks that contain them, so it streams per-chunk
  and never despawns from under you (CDDA-style). **Inline palettes
  resolve** (`palette.rs` registers `type: palette` objects declared in
  a building's own mapgen, e.g. the school's `school_palette`).
- Adding a palette-compatible house is one line in `cdda-files.txt` +
  one in `HOUSE_LAYOUTS`; a one-off adds a `*_template()` + a `specs`
  line (`cdda/building.rs`). The coupling (manifest ↔ Rust registry) is
  still real — the `build.rs`-codegen fix in checklist territory would
  collapse it, and the systematic sweep (checklist #1) is the way past
  hand-picked buildings.
- **Perf (measured, not guessed):** seer's native run now times every
  `app.update()` and reports it (overall + per tour-stop). First numbers
  (200 frames, native, no GPU): steady-state **p50 = 316µs**; standing in
  the school ≈ **0.4ms/frame** — sub-millisecond, negligible. The *only*
  real cost is the region **load frame** (school max **19.5ms**), a
  one-frame hitch when a big building streams in. So "big building =
  heavy" is false for steady-state; the load spike is the only thing
  worth looking at, and even that is inflated here because the tour
  *teleports* (bulk-loads a region) rather than walking one boundary at a
  time. Do not claim perf without a seer `[perf]` line.

## Build / run

Nightly toolchain (edition 2024, `-Z build-std`). CDDA corpus must be
present first (below) or the build fails loudly.

- **wasm (local debug):**
  ```
  cd game
  RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
    cargo +nightly build --target wasm32-unknown-unknown --lib \
    -Z build-std=std,panic_abort
  ```
  CI builds `--release` with `SEER_BUILD_COMMIT` / `SEER_BUILD_TIME` set.
- **native + tests:** `cd game && cargo +nightly test` (88 lib + a few
  integration).
- **web bundle:** `cd game/web && bun run build.ts` → `dist/`
  (`main-<hash>.js`, content-hashed; index.html rewritten to it).
- **imports boundary:** build `crates/seer-imports-check`, run against the
  wasm vs `game/imports.allow` (44 imports).

### CDDA corpus (a dependency, never vendored)

The build needs CDDA mapgen/palette files. They are **not in git**.

- **Nix:** `cd game && nix build .#cdda-src` → set `CDDA_SRC` to the
  result. Pinned by content hash in `game/flake.nix` to release `0.I`.
- **Bare cargo / local:** `cd game && make cdda` (runs
  `tools/fetch-cdda.sh`, sparse-clones `0.I` into gitignored
  `.cdda-src/`). `build.rs` falls back to `.cdda-src` when `$CDDA_SRC` is
  unset.
- Which files: `game/cdda-files.txt` — the single manifest read by
  `build.rs`, the flake, and the fetch script. **Add a building = one
  line there.** Release string: `game/CDDA_RELEASE` (`0.I`). Attribution:
  `game/assets/cdda/ATTRIBUTION.md` (CC-BY-SA 3.0).

## Version / "what's running" (read before touching)

The running binary proves its own identity; no file can lie about it.

- `build_info::COMMIT` / `BUILT_AT` compiled into the wasm from
  `SEER_BUILD_COMMIT` / `SEER_BUILD_TIME`.
- **Watermark** (`src/watermark.rs`) draws the short sha as UI quads,
  top-right, every frame — THE indicator. Not `build-info.json`.
- **Content-hashed bundle** behind `no-store` index.html; **`?v=`
  cache-busted wasm fetch**; **boot-time build-match guard** in `main.ts`
  (see checklist — decide its fate).

## Verification status — BLUNT

Unit-tested green: CDDA assembly/palette resolution, edge placement +
T-junction, glass/ghost split, cut-away scoping (adjacent building stays
solid), jukebox edge, HUD/slider math, music+sfx state, persistence
round-trips, watermark layout.

Confirmed on device by the user: **glass**. The **cut-away, ghost, and
edge-wall** work was visually driven (the artifacts fixed — X-cross
pillars, see-through-from-outside — are only findable by looking), so
it's had eyes on it, but not a systematic pass.

**Not driven end-to-end by anyone:** music toggle + mix-persist across a
reload, both sliders on device, jukebox toggle, ESC/gear close,
watermark glyph legibility at size, ghost + cut-away standing between two
adjacent buildings. The dev agent has no GPU — it verifies no visuals
itself. "Tests pass" is necessary, not sufficient.

## Environment gotchas

- **No GPU** in the dev sandbox — native wgpu render and any visual check
  fail (`No suitable graphics adapter`). Only the user's device confirms
  visuals.
- **`api.github.com` is blocked** by the proxy, so `nix` flake *input*
  resolution via `github:` fails — but the proxy's **git mirror works**
  (`builtins.fetchGit`, plain `git clone` succeed). That's why the CDDA
  hash was verified via `builtins.fetchGit` + `nix hash path`, not a full
  `nix build`, and why `flake.lock` couldn't be generated here.
- **`imports.allow` is enforced** — every wasm↔JS crossing is a
  hand-wired `env.*` import; adding one means editing `imports.allow` +
  `main.ts` + the Rust extern, and CI diffs it.
- **Strict TDD**, **errors are sacred**, **Rust owns the render/UI** —
  see `game/CLAUDE.md` and repo `CLAUDE.md`.

## Operating principles (the user's model — operate by this)

- The running binary is the only source of truth — not metadata files,
  not commit history, not claims.
- Everything the player sees lives in the game, in Rust. Not JS/DOM.
- The loop closes itself: no ambiguity about what's running, and never
  blame the user's cache/refresh.
- **Verify before asserting — about everything** (e.g. `curl` the live
  build + read the Actions run before claiming CI state).
- Don't vendor other people's code; pin it.
- Quality, fully, now — no deferring, no false either/or questions, no
  fake caveats. Drive the actual flow before claiming behavior.

## Key files

- `src/cdda/` — `parse` (JSON walkers), `cells` (char→PropKind +
  wall-line predicate), `placement` (edge-placed walls/windows, roof,
  flood-fill interior), `building` (assembly + registry), `chunks` (world
  placement, index, rotation). `mod.rs` re-exports.
- `src/scene.rs` — `GLASS_SHADER_WGSL`, `GHOST_SHADER_WGSL`, the
  opaque/glass/ghost split + cut-away anchoring.
- `src/render_web.rs` — opaque → glass → ghost → UI passes (wasm).
- `src/render.rs` — native mirror (glass; no ghost — CI PNGs don't need
  it).
- `src/gpu_web.rs` — glass + ghost pipeline/draw imports + wrappers.
- `src/hud.rs`, `src/music.rs`, `src/sfx.rs`, `src/jukebox.rs`,
  `src/bang.rs` — HUD, audio mixes, jukebox, the "!" alert.
- `src/watermark.rs` — in-game commit watermark. `src/build_info.rs` —
  COMMIT / BUILT_AT.
- `src/persist.rs` — position + music + sfx localStorage round trips.
- `web/build.ts`, `web/src/main.ts` — bundle hashing, `?v=` wasm fetch,
  build-match guard.
- `flake.nix`, `CDDA_RELEASE`, `cdda-files.txt`, `build.rs`,
  `tools/fetch-cdda.sh` — the CDDA dependency.
- `imports.allow` — the wasm↔JS boundary.
- `.github/workflows/deploy-game.yml`, `seer.yml` — CI/deploy.
