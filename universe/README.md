# universe

**The only command that matters (same shape as roam):**

```
nix develop -c make wasm-serve
```

Run it from this directory (`universe/`). Open `http://localhost:8085`. The Makefile calls `trunk serve` underneath.

## What this is

Bevy test. No libp2p, no eframe, no roam. Just: does Bevy on WebGL2 attach to an existing `<canvas id="bevy">` and render a clear color.

Layout lifted from `NiklasEi/bevy_game_template`. Stripped of audio, mobile workspace, asset loader, icon embedding.

## Files

- `Cargo.toml` — Bevy with the feature shortcuts NiklasEi uses (`default_app`, `2d_api`, `2d_bevy_render`, `ui_*`, `scene`, `bevy_winit`, `default_font`, `webgl2`). Profile config matches Bevy's official "compile with performance optimizations" recipe.
- `flake.nix` — nix dev shell with `rust` (wasm32 target), `wasm-bindgen-cli`, `trunk`, `sccache` (set as `RUSTC_WRAPPER`).
- `Trunk.toml` — `public_url = "./"`, `port = 8085`.
- `index.html` — minimal page with `<canvas id="bevy">`.
- `src/main.rs` — `App::new()` + `DefaultPlugins.set(WindowPlugin { canvas: "#bevy", … })` + `ClearColor` + `Camera2d`.
- `.gitignore` — excludes `target/`, `dist/`, `pkg/`.

## Constraints

- **Nix flakes only see git-tracked files.** If you add a new file here, `git add` it or `nix develop` won't see it.
- **trunk lives in `universe/flake.nix`, not in `roam/flake.nix`.** The dev shell here is different from roam's.
- **`bevy/dynamic_linking` is in the Cargo.toml `dev` feature but does nothing on wasm32.** Native-only speedup; wasm always statically links.
