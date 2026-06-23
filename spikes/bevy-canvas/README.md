# bevy-canvas-spike

**The only command that matters (same shape as roam):**

```
nix develop -c make wasm-serve
```

Run it from this directory (`spikes/bevy-canvas/`). Open `http://localhost:8085`. Underneath, the Makefile calls `trunk serve` — that's a hidden implementation detail; the user-facing shape matches `roam/`'s convention so muscle memory carries.

Success = magenta canvas. That's the v0.5.0 canvas-attach proof per `roam/docs/adr/0003-bevy.md`.

First cold compile: 3–5 min (Bevy 0.18 + dependencies on wasm32, debug profile). Subsequent saves rebuild in 5–30s.

## What this is

Sealed Bevy 0.18 canvas-attach test. No libp2p, no eframe, no roam. Just: does Bevy on WebGL2 attach to an existing `<canvas id="bevy">` and render a clear color. If yes, roam's v0.5.1 (port `ui/mod.rs` to `bevy_ui`, drop eframe) can begin.

Layout lifted from `NiklasEi/bevy_game_template` (1.1k stars, canonical Bevy+trunk starter). Stripped of audio, mobile workspace, asset loader, icon embedding — none needed for canvas attach.

## Files

- `Cargo.toml` — Bevy 0.18 with the feature shortcuts NiklasEi uses (`default_app`, `2d_api`, `2d_bevy_render`, `ui_*`, `scene`, `bevy_winit`, `default_font`, `webgl2`). Profile config matches Bevy's official "compile with performance optimizations" recipe.
- `flake.nix` — nix dev shell with `rust` (wasm32 target), `wasm-bindgen-cli`, `trunk`, `sccache` (set as `RUSTC_WRAPPER`).
- `Trunk.toml` — `public_url = "./"`, `port = 8085` (distinct from roam's `8083` and the common-conflict `8080`).
- `index.html` — minimal page with `<canvas id="bevy">`.
- `src/main.rs` — `App::new()` + `DefaultPlugins.set(WindowPlugin { canvas: "#bevy", … })` + magenta `ClearColor` + `Camera2d`.
- `.gitignore` — excludes `target/`, `dist/`, `pkg/`.

## Constraints

- **Nix flakes only see git-tracked files.** If you add a new file here, `git add` it or `nix develop` won't see it.
- **trunk lives in this spike's `flake.nix`, not in `roam/flake.nix`.** That's why the command is run from this directory — the dev shell for the spike is different from roam's.
- **`bevy/dynamic_linking` is in the Cargo.toml `dev` feature but does nothing on wasm32.** Native-only speedup; wasm always statically links. The recipe is preserved for when/if anyone runs the spike on native.

## Why this lives at the repo root in `spikes/`, not inside `roam/`

The spike has its own Cargo + nix + serving stack. Folding into `roam/`'s build graph (one wasm bundle alongside libp2p + eframe) was tried and burned hours on feature curation. Sealed is faster and cleaner. Per the conversation that produced ADR 0003 + this directory.
