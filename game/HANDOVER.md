# game — handover

Working notes for continuing this branch. Blunt on purpose. A snapshot,
so re-verify live-vs-HEAD and re-read the checklist before trusting it.

Branch: **`stamp-template`**, pushed to `origin`. Live at game.sbvh.nl
(check `curl -s https://game.sbvh.nl/build-info.json` vs
`git rev-parse --short HEAD`; the in-game watermark top-right is the
ground truth). Snapshot at tip **`3b25e8f`**.

> **Deploy + seer status (verified at this snapshot).** The flake-hash
> break is fixed (decoupled fetch, `38b0ef5`) and the **deploy recovered**
> — live reached HEAD again at `94c8696`. **seer** was separately *green
> but dark*: it ran on every push yet measured **nothing** for this whole
> branch (see the lesson below). Fixed and **verified live at `3b25e8f`**:
> `seer.sbvh.nl/perf/history.json` now carries `3b25e8f` (frames=300,
> verdict PASS), per-sha permalinks resolve. Still re-verify live-vs-HEAD
> before trusting any of this — a snapshot goes stale fast.

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

- [x] **Confirm the deploy recovered (do this first).** Verified: live
  reached HEAD (`9ef9b53`) before the recent batch; `nix build
  .#cdda-src` builds cleanly locally; every push since has been green in
  both `deploy-game` and `seer`.
- [x] **Coverage report over the full 0.I mapgen tree.** Landed as
  `game/src/bin/cdda_coverage.rs`. First pass on 0.I: 845 mapgen files,
  6284 distinct om_terrains → **1675 resolved** (330K props total), 4498
  empty (mostly roof-only entries + palette gaps), 111 skipped, **0
  importer failures**. Top unhandled features by hit count:
  `place_nested` (654), `place_loot` (418), `place_monster` (372),
  `place_monsters` (331), `place_vehicles` (305). Empties are the next
  real lever — either the mapgen is roof-only (fine) or the palette
  chars aren't yet mapped to any prop.
- [x] **`build.rs` basename-collision guard.** Asserts basename
  uniqueness before copying; fails loudly with both offenders in the
  message. Tests pass; not yet stress-tested with an actual same-name
  collision in the manifest.
- [x] **Determinism golden-master.** `Template::stable_digest` is an
  FNV-1a of an explicit per-prop byte serialisation (f32 LE offsets,
  pinned u8 tag per `PropKind`, colour tag + straight bytes). Test pins
  the 28 shipped-template digests locally; **not yet cross-platform
  verified** — the hash is designed to be platform-stable, but a Linux
  CI run against these pins is the real proof.
- [x] **Commit `game/flake.lock`.** Committed via `nix flake lock`;
  nightly rust, rust-src, wasm target now pin.
- [x] **`.cdda-src` staleness guard.** `fetch-cdda.sh` stamps
  `.cdda-src/.rev` with `CDDA_RELEASE`; `build.rs` fails if it drifts.
  Nix path is hash-pinned already, skips this check.
- [x] **`tools/fetch-cdda.sh` integrity.** Pinned expected commit in
  `game/CDDA_COMMIT`; script fails if the fetched HEAD doesn't match.
- [x] **Decide the JS build-match guard.** Decision: **KEEP**.
  CloudFront's Managed-CachingOptimized has `QueryStringBehavior:"none"`
  (verified on distribution `E398MD1MAZIFP7`), so `?v=` only ever busted
  the browser cache. Deploy invalidation refreshes the CDN; boot-time
  build-match guard covers the browser cache slip. Removed the
  redundant `?v=` query string (URL params banned repo-wide).
- [x] **Unify `Music` resource access.** Both `hud` and `jukebox` now
  take `Option<ResMut<Music>>` (and `Option<ResMut<SfxMix>>` for the
  sliders), so setup-order changes never explode the frame.
- [ ] **Implement `place_nested`.** The coverage report's measured #1
  blocker — **654 hits**, more than any other unhandled feature
  (place_loot 418, place_monster 372, place_monsters 331, place_vehicles
  305). It's the biggest lever on the 4498 "empty" resolves, and it lets
  the hand-authored `shed.json` be **deleted** (CDDA's sheds are nested
  pieces, not standalone). Start in `cdda/placement`.
- [ ] **Publish the native `[perf]` frame-time to the seer site.** THE
  measurement Brandon cares about ("1 measurement: real live
  performance") is produced by the `game-native` run but only uploaded as
  a CI *artifact* (`seer.yml` `upload-artifact`), never to
  `seer.sbvh.nl/perf/…` — so it's measured and invisible on the site.
  Upload `game-native.log` (or a parsed `perf.json`) to `/perf/<sha>/` +
  `/perf/latest/` beside the frame PNGs, and surface it in the viewer.
- [ ] **Drive the un-verified flows on device** (see Verification).
  Includes **confirm the purple jukebox actually renders** — Brandon
  reported on 07-13 he "never found the purple jukebox you said you would
  create"; the code exists (`src/jukebox.rs`, `scene.rs`) but on-device
  presence is unconfirmed. Plus music/SFX persist across reload, both
  sliders, ESC/gear close, ghost+cut-away between two adjacent buildings.
- [ ] **Open a PR.** Squash the vendoring churn and scrub the CC-BY-SA
  JSON blobs from history first.
- [ ] **(Brandon, manual — not the agent) Write the perf norm into
  `game/CLAUDE.md`.** It says the game is "observed by seer" but never
  states the rule, so it got missed (see lesson below). The intent to
  encode: *performance is only what seer's `[perf]` measures; never infer
  it from counts (props/bytes/instances); a big number is not a slow
  frame.* Likely a minor adjustment — the exact wording is yours; the
  agent should not edit that file.
- [x] **Make the tour walk the last stretch** into a building. On each
  new stop, the player teleports 800 units north of the target then
  walks in at `KEYBOARD_SPEED`, so chunks stream in one boundary at a
  time. Not yet verified against the seer perf history — need a run to
  confirm the load spike moved to a real per-boundary number.
- [x] **Get frame time from the real browser** (rAF delta → seer). Landed
  in `d191031`: rAF-to-rAF delta captured every tick, p50/… emitted
  through seer. Not yet confirmed showing up in the seer perf history on
  device — needs a real browser run against the viewer to verify the
  number lands, and pairs with "publish the native `[perf]`" above so both
  the CI-native and on-device browser numbers are visible.

### Lesson — the perf failure mode (don't repeat it)

A calibrated measurement tool (seer) already existed and I wasn't
operating from it — `game/CLAUDE.md` literally says "Observed by seer"
and I still reached for inference. The failure mode: I counted things
(props, bytes, instances), saw a big number, and called it "heavy" /
"perf concern" — dressed as an "honest flag." **Counts are not
performance.** The measured reality was the opposite: standing in the
"3,755-prop" school costs ~0.4ms/frame (steady state), negligible; the
only real cost is a one-frame load hitch. seer is the *only* way we
actually know. Rule going forward: no perf claim without a seer `[perf]`
line — and if seer can't measure it yet, the task is to extend seer, not
to guess.

### Lesson — the deploy went stale under a green local build

For 9 commits the deploy + seer were **broken and I didn't know**, because
after asserting "live == HEAD" early on I stopped checking it and kept
piling commits on. Root cause: the flake sparse-fetched *exactly*
`cdda-files.txt`, so the content hash was a function of the file list;
adding a building changed the hash, `nix build .#cdda-src` failed, and
that's the first step of both deploy-game and seer. Local `cargo` builds
stayed green because they use the `.cdda-src` fetch path, which **doesn't
verify the hash** — so nothing surfaced the break here. Two rules from
this: (1) after any deploy-affecting push, **verify `build-info.json` ==
HEAD** — a green local build is not a green deploy; (2) a build path that
skips the integrity check the real path enforces will hide exactly this
class of break. Fixed at `38b0ef5` by decoupling the fetch (fixed
subtrees) from the manifest, so the hash no longer moves on corpus edits.

### Lesson — seer was green but dark (measured nothing for the whole branch)

seer looked alive — it ran on every push, the jobs were green, `frame.png`
and a `seer-host.log` kept uploading — but it was measuring **nothing**.
The live `perf/latest/seer-host.log` said it plainly:

```
[host] instantiating
Error: unknown import: `env::game_gpu_render_glass` has not been defined
```

Every boundary crossing this branch added — glass/ghost pipeline+render,
position/music/sfx persistence, audio volume: **11 in total** — went into
`imports.allow` and the game's externs but was **never mirrored into
seer-host's linker**. wasmtime rejects the module at instantiation on the
first missing import, so seer-host ran zero frames and wrote no
summary/metrics/history JSON. That's why `history.json` froze at Jul 10
while the branch kept moving. Two compounding causes:

1. **A swallowed error.** The run step piped seer-host through `| tee`, so
   the pipe returned tee's exit 0 and a hard crash left the job **green**.
   Errors are sacred — a buried one rots for days. Fixed: the step now
   preserves seer-host's exit code (`PIPESTATUS`) + emits `::error::`, so a
   host crash turns the job **red**.
2. **No conformance check.** `seer-imports-check` proves game.wasm's
   imports ⊆ `imports.allow`, but nothing proved seer-host's linker ⊇
   `imports.allow`. Added that test
   (`linker_satisfies_every_allowed_import`). The two now compose: game's
   imports ⊆ allow ⊆ host → instantiation can never again fail on a missing
   import, and it's caught by `cargo test` at the source, not as stale S3
   data days later.

Rule from this: **a green pipeline is not a live measurement — read the
artifact it produced.** `history.json` had the wrong newest sha and
`seer-host.log` was 189 bytes; both were visible from a plain `curl`. The
job colour said nothing. Fixed + verified live at `3b25e8f`.

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
- **Boundary:** 44 hand-wired `env.*` imports. **Tests:** 98 lib green,
  clippy silent.
- **seer:** runs on every push to any branch + nightly (paths gate
  removed). Its native run now **measures real per-frame time** and
  **tours the world** (teleports through a school, house, campsite,
  forest) so it encounters variety instead of empty forest. **seer-host's
  linker now covers all `imports.allow` crossings** (was missing 11 — glass/
  ghost/persist/audio — so game.wasm failed to instantiate; see lesson),
  guarded by `linker_satisfies_every_allowed_import`, and a host crash now
  turns the job red instead of hiding behind `tee`. Verified live: the wasm
  run is publishing per-sha summary/metrics/history again.
- **CDDA fetch is decoupled from the manifest** (`38b0ef5`): the flake +
  `fetch-cdda.sh` fetch the fixed `mapgen` + `mapgen_palettes` subtrees,
  `build.rs` picks which files to embed from `cdda-files.txt`. The
  flake hash only moves on a `CDDA_RELEASE` bump — adding a building
  never touches it.
- **Corpus:** **7** CDDA buildings — garage, houses 01–04 (each ×6
  palette variants), the daycare, and the **school** (the first
  multi-tile building: a 3×3 / 72×72 grid) + our shed = 28 templates.
- **Multi-tile buildings work** (`chunk.rs`): a building's props are
  distributed to the chunks that contain them, so it streams per-chunk
  and never despawns from under you (CDDA-style). **Inline palettes
  resolve** (`palette.rs` registers `type: palette` objects declared in
  a building's own mapgen, e.g. the school's `school_palette`).
- Adding a palette-compatible house is one line in `cdda-files.txt` +
  one in `HOUSE_LAYOUTS` (no flake edit, no hash re-pin — the fetch is
  decoupled); a one-off adds a `*_template()` + a `specs` line
  (`cdda/building.rs`). The remaining coupling (manifest ↔ Rust registry)
  is a `build.rs`-codegen fix away, and the systematic sweep (checklist
  #1) is the way past hand-picked buildings.
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
- **native + tests:** `cd game && cargo +nightly test` (98 lib + a few
  integration).
- **web bundle:** `cd game/web && bun run build.ts` → `dist/`
  (`main-<hash>.js`, content-hashed; index.html rewritten to it).
- **imports boundary:** build `crates/seer-imports-check`, run against the
  wasm vs `game/imports.allow` (44 imports).

### CDDA corpus (a dependency, never vendored)

The build needs CDDA mapgen/palette files. They are **not in git**.

- **Nix:** `cd game && nix build .#cdda-src` → set `CDDA_SRC` to the
  result. Pins the `mapgen` + `mapgen_palettes` subtrees at `0.I` by
  content hash in `game/flake.nix` (fixed set — not `cdda-files.txt`).
- **Bare cargo / local:** `cd game && make cdda` (runs
  `tools/fetch-cdda.sh`, sparse-clones the same subtrees at `0.I` into
  gitignored `.cdda-src/`). `build.rs` falls back to `.cdda-src` when
  `$CDDA_SRC` is unset.
- Which files to embed: `game/cdda-files.txt` — read by `build.rs` (the
  fetch grabs whole subtrees; this manifest only selects what's compiled
  in). **Add a building = one line here** (+ the registry), no flake
  touch. Release string: `game/CDDA_RELEASE` (`0.I`). Attribution:
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
