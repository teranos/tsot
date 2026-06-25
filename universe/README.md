# universe

Cells-stage game prototype. WASD + drag through tide-pool water. Eat algae (smaller cells) to grow. Camera follows. Press `/` in-canvas for the diagnostic drawer (FPS, captured errors).

Deployed at https://universe.sbvh.nl/ via CI on push to `bevy` or `master` (paths filter `universe/**`). No local dev — Bevy compile cost lives in CI, not on this machine. See `.github/workflows/deploy-universe.yml`.

The eventual direction (deferred — see `roam/README.md` "what i want"): multi-cell tide pool over libp2p, germ-line identity persisting across cell deaths, Spore-style stage progression, ~60-min heat-death universe.

## Files

- `Cargo.toml` — Bevy (pinned), wasm-bindgen, js-sys. `crate-type = ["cdylib", "rlib"]`. Profile recipe: `opt-level = 1` self, `opt-level = 3` deps.
- `flake.nix` — nix dev shell with rust (wasm32 target), `wasm-bindgen-cli`, `sccache`.
- `Makefile` — `make wasm` runs `cargo build --release --lib` + `wasm-bindgen --target web`. CI calls this.
- `index.html` — `<canvas id="bevy">` + ES module loading the wasm-bindgen output.
- `src/lib.rs` — App, components, systems. Sacred-error pipeline (panic hook + LogPlugin custom_layer → in-canvas drawer). Cells: PlayerCell, Algae, WaterParticle, Tethered (halo + nucleus follow), Dying (eat animation).
- `src/main.rs` — native entry that calls `lib::run()`.
- `BEVY.md` — open decisions, references, Bevy version bump trigger.

Nix flakes only see git-tracked files. New files need `git add` before `nix develop` sees them.
