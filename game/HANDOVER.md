# game — handover

Working notes for continuing this branch in a fresh session. Blunt on
purpose: what's real, what's tested, what's never been looked at.

## Known defects & risks (read this first)

Each has a location and a fix. Ranked by how likely it is to bite. These
are the concrete things to improve — not vibes.

1. **`build.rs` silently clobbers on basename collision.**
   `game/build.rs` copies every `cdda-files.txt` path to
   `OUT_DIR/cdda/<basename>` (the `basename(entry.path)` copy loop). The
   8 current basenames are unique, so it works today. Add a file whose
   basename collides (a second `roof.json`, another `house01.json`) and
   the second copy overwrites the first — `include_str!` then embeds the
   wrong bytes with no error. Fix: assert basenames are unique in
   `build.rs`, or copy under the full relative path.

2. **No `game/flake.lock`.** `game/flake.nix` has no committed lock; the
   repo root and `roam/` both do. So `nix build .#cdda-src` resolves
   nixpkgs/rust-overlay fresh every run — the corpus bytes stay
   hash-pinned but the *builder toolchain* floats. Fix:
   `cd game && nix flake lock` and commit it. (Not done here: the sandbox
   blocks `api.github.com`, which flake input resolution needs; the git
   mirror works but nixpkgs is too big to clone that way.)

3. **`Music` resource access is inconsistent.**
   `hud::hud_input_system` takes `ResMut<Music>` (panics if the resource
   is missing); `jukebox::jukebox_proximity_system` takes
   `Option<ResMut<Music>>` (skips). `setup_music` always inserts it, so
   it's fine now, but the two contracts diverge the moment ordering or a
   feature flag changes. Pick one — likely `Option<ResMut>` everywhere.

4. **`.cdda-src` staleness is invisible.** `build.rs` falls back to
   `game/.cdda-src` when `$CDDA_SRC` is unset, but nothing ties that
   cache to `CDDA_RELEASE`. Bump `CDDA_RELEASE`, forget to re-run
   `make cdda`, and you compile the *previous* release silently. Fix:
   record the fetched rev (e.g. `.cdda-src/.rev`) and make `build.rs`
   fail if it doesn't equal `CDDA_RELEASE`.

5. **`tools/fetch-cdda.sh` verifies nothing.** The nix path is
   hash-pinned; the script path (used by `make cdda` and any non-nix
   build) clones the `0.I` tag and compiles whatever it gets — a moved
   tag or bad mirror goes undetected. Fix: after fetch, sha the files and
   compare to a committed checksum, or drop the script path and require
   nix.

6. **`?v=<commit>` wasm cache-bust may not reach the CDN.** `main.ts`
   fetches `/game.wasm?v=<commit>`. This reliably busts the *browser*
   cache; whether it busts *CloudFront* depends on whether the query
   string is in the distribution's cache key — unconfirmed. If it isn't,
   the deploy's explicit `/game.wasm` invalidation is the real refresh
   and the boot-time build-match guard is the backstop. Action: check the
   distribution's cache-key config; don't rely on `?v=` blindly.

7. **The JS build-match guard: decide, don't leave it ambiguous.**
   `main.ts` reads the wasm's own commit (exports `build_commit_ptr/len`)
   and halts if it differs from the commit baked into the bundle. With
   the Rust watermark showing the real commit and the wasm cache-busted,
   it's a narrow backstop. Keep it as a deliberate hard-stop, or delete
   it — currently it's there by momentum.

8. **Watermark glyphs are unseen at size.** `watermark.rs::glyph()` has
   hand-drawn 3×5 bitmaps for `0-9 a-f u n k o w`; `n/k/w` are cramped in
   3 px. The sha renders (confirmed on device), but per-glyph legibility
   isn't checked. If a character reads wrong, edit that one entry.

## Not a defect — leave it alone

- **Glass looks correct — user-verified visually.** The glass pass draws
  panes unsorted with depth-write off, which is *theoretically*
  order-dependent. In practice it looks right and is not a problem. Only
  if you ever see wrong blending where many panes overlap at once, add a
  back-to-front sort in `scene::snapshot_to_glass_instances`. Do not
  pre-emptively "fix" this.

## Also open (not code defects)

- No PR opened yet.
- History is noisy (CDDA went vendor → de-vendor → pin across many
  commits) and still contains the CC-BY-SA JSON blobs in old commits.
  Squash / scrub before opening a PR.
- Of the visual features, only **glass** has been confirmed on-device.
  Music toggle, settings slider, jukebox toggle, and the wall cut-away
  feel have not been driven end-to-end — verify them next.

## How this went wrong (so it doesn't repeat)

- Green unit tests and confident commit messages were treated as proof
  the running thing works. They only prove it compiles. Drive the actual
  flow before claiming behavior.
- State was asserted without checking (said CI "may be broken" without
  opening Actions). Verify first: `curl -s .../build-info.json` vs
  `git rev-parse --short HEAD`, and read the Actions run.
- Anything the user sees belongs in the Rust render, not a JS/DOM
  overlay. The commit readout was built as a JS badge first — wrong.
- Don't hand the user implementation either/or questions; make the call
  and state it.

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

Confirmed on-device by the user: **glass windows render fine.**

**Not driven end-to-end by anyone yet:**
- Music toggle / settings slider / jukebox toggle behaving on a device.
- The watermark glyphs at real size (font is math-checked only).
- The wall cut-away feel after tightening the threshold.

The dev agent has no GPU, so it verified none of the visuals itself —
they came from the user's screenshots. Treat "tests pass" as necessary,
not sufficient.

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
