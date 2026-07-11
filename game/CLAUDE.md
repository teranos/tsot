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

## No URL flags, no hidden features

Banned. No `?p2p=laye`, no `?sound=off`, no `?proxy=off`, no `?debug`,
no hidden features gated on query params. The user does not type
flags. If a variation needs to exist, it lives on a branch and the
picker navigates to it.

## In-game UI is Bevy — not HTML, not JS

Any control the player interacts with while playing lives in Rust.
D-pad, sound toggle, HUD, inventory — Bevy resource + UI overlay
pipeline + touch hit-test. HTML overlays are for app-meta UI only
(loading screen, version picker) and even those are candidates for
Bevy once the pipeline supports them.
