# game — handover

Working notes for continuing this branch in a fresh session. Blunt on
purpose: what's real, what's tested, what's never been looked at.

## Branch

`stamp-template`. All work below is on it and pushed to
`origin/stamp-template`. Latest deployed commit is live at
game.sbvh.nl (check: `curl -s https://game.sbvh.nl/build-info.json`
vs `git rev-parse --short HEAD`).

## What landed this stretch

- **Glass windows** — real alpha-blended transparency. Windows resolve
  to `PropKind::Window*` and render in a dedicated blended pass
  (depth-tested, depth-write off) *between* the opaque world and the UI.
  New hand-wired imports `game_gpu_render_pipeline_create_glass` +
  `game_gpu_render_glass`. Mirrored in native `render.rs`.
- **HUD** — left music toggle, settings gear, settings panel with a
  live volume slider, all drawn through the UI-overlay quad pipeline.
- **Purple jukebox** — in-world prop near spawn; walking into its radius
  toggles the music (edge-triggered).
- **Music** — one `music::Music` resource; volume/mute applied live via
  the new `game_audio_set_volume` import (GainNode, no stop/reload).
- **CDDA as a pinned dependency** — no third-party JSON in git anymore
  (see below).
- **Build-loop closure** — content-hashed JS bundle, cache-busted wasm
  fetch, boot-time build-match guard, and an **in-game commit
  watermark drawn in Rust** (`watermark.rs`), top-right.

## Build / run

Nightly toolchain required (edition 2024, `-Z build-std`).

- **wasm (local debug), the way used here:**
  ```
  cd game
  RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
    cargo +nightly build --target wasm32-unknown-unknown --lib \
    -Z build-std=std,panic_abort
  ```
  CI builds `--release` (see `.github/workflows/deploy-game.yml`) with
  `SEER_BUILD_COMMIT` / `SEER_BUILD_TIME` env set.
- **native + tests:** `cd game && cargo +nightly test` (76 lib + a few
  integration; 78 with the new watermark tests).
- **web bundle:** `cd game/web && bun run build.ts` → `dist/`
  (`main-<hash>.js`, hashed; index.html rewritten to it).
- **imports boundary:** build `crates/seer-imports-check`, run it against
  the wasm vs `game/imports.allow` (currently 39 imports).

### CDDA corpus (a dependency, never vendored)

The build needs the CDDA mapgen/palette files. They are **not in git**.

- **Nix (CI + hermetic):** `cd game && nix build .#cdda-src` → set
  `CDDA_SRC` to the result; `build.rs` reads it. Pinned by content hash
  in `game/flake.nix` to the `0.I` stable release.
- **Bare cargo / local:** `cd game && make cdda` (runs
  `tools/fetch-cdda.sh`, sparse-clones the pinned release into
  gitignored `.cdda-src/`). `build.rs` falls back to `.cdda-src` when
  `CDDA_SRC` is unset; missing corpus fails the build loudly.
- Which files: `game/cdda-files.txt` (the single manifest, read by
  `build.rs`, the flake, and the fetch script). Add a building = one
  line there. Pinned release string: `game/CDDA_RELEASE` (`0.I`).
- Attribution: `game/assets/cdda/ATTRIBUTION.md` (CC-BY-SA 3.0).

## Version / "what's running" model (read this before touching it)

The whole point: the running binary proves its own identity; no file can
lie about it.

- `build_info::COMMIT` / `BUILT_AT` are compiled into the wasm from
  `SEER_BUILD_COMMIT` / `SEER_BUILD_TIME`.
- **Watermark** (`src/watermark.rs`) — Rust reads `build_info::COMMIT`
  and draws the short sha as UI-overlay quads, top-right, every frame.
  This is THE version indicator. Not `build-info.json` (a separate file
  that skewed during the cache incident), not the loading bar.
- **Content-hashed bundle** (`web/build.ts`) — `main-<hash>.js` behind a
  `no-store` index.html; a browser can't serve a stale bundle against a
  new wasm.
- **Cache-busted wasm** — `main.ts` fetches `/game.wasm?v=<commit>`.
- **Build-match guard** — `main.ts` reads the wasm's own commit (exports
  `build_commit_ptr/len`) and refuses to boot if it differs from the
  commit baked into the bundle. NOTE: possibly redundant now given the
  watermark + cache-bust; a decision is pending (see open items).

## Verification status — BLUNT

Unit-tested and green: CDDA assembly + palette resolution, glass/opaque
instance separation, jukebox proximity edge, HUD layout/slider math,
music state machine, wall cut-away threshold, watermark font/layout.

**Never rendered or looked at by the dev agent — no GPU in the sandbox:**
- Glass windows actually appearing as glass in a building. UNCONFIRMED.
- Music toggle / settings slider / jukebox toggle behaving on a device.
- The watermark glyphs at real size (font is math-checked only).
- The wall cut-away feel after tightening.

Everything visual has only ever been confirmed by the user's phone
screenshots. Treat "tests pass" as necessary, not sufficient.

## Open items (ranked)

**Must-do (blocks a real review):**
1. Get eyes on the actual pixels — glass first. Nothing visual is
   verified by the agent.
2. **Glass transparency is unsorted** — blended panes drawn in arbitrary
   order with depth-write off → order-dependent compositing; windows on
   two walls can blend wrong. Needs back-to-front sort (or per-pane
   depth strategy). Real bug.
3. **No `game/flake.lock`** while root + `roam/` commit one — unlocked
   inputs. Commit a lock (`nix flake lock` in `game/`).
4. **`build.rs` basename-collision guard** — it flattens manifest paths
   to `OUT_DIR/cdda/<basename>` with no uniqueness check; a future
   same-basename file silently clobbers.

**Should-do:**
- Decide the JS build-match guard's fate (keep as hard-stop backstop, or
  remove now that the watermark + cache-bust cover it).
- `tools/fetch-cdda.sh` trusts what it downloads (no checksum); the nix
  path is hash-verified but the script path isn't.
- Stale `.cdda-src` is used silently if `CDDA_RELEASE` changes without a
  re-fetch — no staleness check.
- History carries the old CC-BY-SA blobs (added pre-de-vendor) and is
  noisy (vendor → re-vendor → de-vendor churn). Squash before a PR.

**Not started:** no PR opened. Glass appearance in a house unconfirmed.

## Environment gotchas

- **No GPU** in the dev sandbox — native wgpu render and any visual
  check fail here (`No suitable graphics adapter`). Only the user's
  device confirms visuals.
- **`api.github.com` is blocked** by the sandbox proxy, so `nix` flake
  *input* resolution (nixpkgs/flake-utils via `github:`) fails here —
  but the proxy's **git mirror works**, so `builtins.fetchGit` and plain
  `git clone` of github repos succeed. This is why the CDDA hash was
  verified via `builtins.fetchGit` + `nix hash path`, not a full
  `nix build`.
- **`imports.allow` is enforced** — every wasm↔JS crossing is a
  hand-wired `env.*` import; adding one means editing `imports.allow` +
  `main.ts` + the Rust extern, and CI diffs it.
- **Strict TDD**, **errors are sacred** (surface at the cursor, never
  swallow), **Rust owns the render** — see `game/CLAUDE.md` and repo
  `CLAUDE.md`.

## User's mental model (operate by this)

- The running binary is the only source of truth — not metadata files,
  not commit history, not claims.
- Features live in the game, in the Rust. Not JS/DOM overlays.
- The loop must close itself: no ambiguity about what's running, and
  never blame the user's cache/refresh.
- Verify before asserting — about everything.
- Don't vendor other people's code; pin it.
- Quality, fully, now — no deferring, no false either/or questions, no
  fake caveats.

## Key files

- `src/watermark.rs` — in-game commit watermark (UI quads).
- `src/build_info.rs` — COMMIT / BUILT_AT.
- `src/scene.rs` — `GLASS_SHADER_WGSL`, glass/opaque split
  (`snapshot_to_glass_instances`), roof/near-wall cut-away.
- `src/render_web.rs` — world → glass → UI passes (wasm).
- `src/render.rs` — native mirror incl. glass pass.
- `src/gpu_web.rs` — glass pipeline/draw imports + wrappers.
- `src/hud.rs`, `src/music.rs`, `src/jukebox.rs` — HUD/music/jukebox.
- `web/build.ts`, `web/src/main.ts` — bundle hashing, wasm fetch,
  build-match guard.
- `flake.nix`, `CDDA_RELEASE`, `cdda-files.txt`, `build.rs`,
  `tools/fetch-cdda.sh` — CDDA dependency.
- `imports.allow` — the wasm↔JS boundary.
- `.github/workflows/deploy-game.yml`, `seer.yml` — CI/deploy.
