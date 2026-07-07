# game

Bevy ECS + wgpu (native) + wasm32 target for browser at game.sbvh.nl.
Observed by seer at seer.sbvh.nl.

## The boundary is the review surface

No wasm-bindgen. No js-sys, web-sys, wasm-bindgen-futures.

Every wasm↔browser crossing is a hand-wired env.* import declared
in `imports.allow`. Any tool that hides the boundary defeats the
review.

Three gates enforce it:

- `imports.allow` + `crates/seer-imports-check` — fails CI when
  game.wasm's actual import section drifts from the allow-list.
  wasm-bindgen adds hundreds of `__wbg_*` / `__wbindgen_*` entries;
  the drift shows in a PR diff before merge.
- `deny.toml` + `cargo deny check` — bans the wasm-bindgen crate
  family at Cargo dep-resolution time. Fails before compile.
- Human review + this file — a future contributor (or Claude
  session) reads this before proposing a workaround.

## Porting from rave / roam

Any rave or roam module whose implementation uses wasm-bindgen
(identity via IndexedDB, libp2p in-browser via bevy-libp2p,
bevy_pbr, bevy_audio, bevy_winit) is NOT a verbatim port. The
operation reshapes to either:

- Hand-wired env.* imports on the wasm side + a small JS shim +
  (for network) a native game-proxy that speaks libp2p to relaye.
- Compute-only wasm + JS-side rendering (canvas / WebGPU driven
  from JS reading wasm memory).

The point is the same as seer's founding call: the seam between
worlds is a first-class thing to inspect, not something to hide
behind macro-generated glue.
