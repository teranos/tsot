# game

Wasm32 for browser at game.sbvh.nl. Observed by seer at
seer.sbvh.nl.

## No wasm-bindgen

Rave's leak was impossible to find through the browser. The seam
between wasm and the browser has to stay inspectable.

Every crossing is a hand-wired env.* import in `imports.allow`.
Enforced by `crates/seer-imports-check`.

## Browser render

Rust owns the render pipeline.

See [Architecture.md](./Architecture.md) for the artifact axioms.
