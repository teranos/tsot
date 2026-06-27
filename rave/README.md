# rave

Bevy + libp2p rave party. Forked from `universe/` at its cells-stage prototype as the starting point. The direction is peers in one shared room over libp2p.

Deployed at https://rave.sbvh.nl/ via CI on push to `rave` or `master` (paths filter `rave/**`). No local dev — Bevy compile cost lives in CI, not on this machine. See `.github/workflows/deploy-rave.yml`.

## Files

- `Cargo.toml` — Bevy (pinned), wasm-bindgen, js-sys. `crate-type = ["cdylib", "rlib"]`. Profile recipe: `opt-level = 1` self, `opt-level = 3` deps.
- `flake.nix` — nix dev shell with rust (wasm32 target), `wasm-bindgen-cli`, `sccache`.
- `Makefile` — `make wasm` runs `cargo build --release --lib` + `wasm-bindgen --target web`. CI calls this.
- `index.html` — `<canvas id="bevy">` + ES module loading the wasm-bindgen output.
- `src/lib.rs` — App, components, systems. Sacred-error pipeline (panic hook + LogPlugin custom_layer → in-canvas drawer). Cells: PlayerCell, Algae, WaterParticle, Tethered (halo + nucleus follow), Dying (eat animation).
- `src/main.rs` — native entry that calls `lib::run()`.
- `BEVY.md` — open decisions, references, Bevy version bump trigger.

Nix flakes only see git-tracked files. New files need `git add` before `nix develop` sees them.
