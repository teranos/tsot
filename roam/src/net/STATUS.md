# roam/src/net — provider seam status

## Current architecture (2026-06-17)

The `NetworkProvider` trait (`mod.rs`) is the seam. Two concrete
implementations live in this directory:

- `js_libp2p.rs` — `JsLibp2pProvider`. Drives the JS-side libp2p
  instance through five thin callbacks defined in
  `assets/src/net-shim.js`: `attach`, `publish`, `subscribe`,
  `unsubscribe`, `selfPeerId`, `drainEvents`. Browser-only.
- `rust_libp2p.rs` — `RustLibp2pProvider`. Direct rust-libp2p in
  wasm32. Real `Swarm` with `websocket-websys` + `webrtc-websys`
  transports, gossipsub + identify + ping behaviours,
  `wasm_bindgen_futures::spawn_local` driver task. On non-wasm32
  builds, a `new_stub` constructor exists so trait surface stays
  exercised by unit tests.

## Runtime selection

`assets/src/js-bridge.js` reads `?provider=` from the URL:

- `?provider=js` (or absent): `JsLibp2pProvider` is constructed
  through `roam_net_init` after the JS libp2p instance is created.
  **This is the production default.**
- `?provider=rust`: `roam_net_init_rust_libp2p(JSON.stringify(bootstrapList))`
  is called instead, and the JS libp2p init is skipped via the
  `SkipLibp2pInit` sentinel. This branch only exists when the wasm
  binary was built with the `rust-libp2p` Cargo feature.

## Build feature flag

`Cargo.toml`:

```
[features]
default = ["rust-libp2p"]
rust-libp2p = ["dep:libp2p", "dep:wasm-bindgen-futures", "dep:futures", "dep:getrandom_0_3"]
```

Default-features-on means the wasm binary includes the
`RustLibp2pProvider` code path. `make wasm` produces a binary that
can switch at runtime by URL flag. `cargo build --no-default-features`
would drop the rust-libp2p code entirely (no `make wasm-js-only`
target wired today — would need adding if a JS-only bundle is ever
required).

## Phase status (from rust_libp2p.rs)

- **3a** — Trait + stub: done.
- **3b.1** — Deps pinned, both feature configs link: done.
- **3b.2** — Real Swarm + driver task + JS bridge wiring: in file.
- **3b.3** — End-to-end parity test (rust provider talking to js
  provider through the deployed relay) + dial bootstrap relay
  semantics: **NOT VERIFIED**. The `?provider=rust` switch will
  call into the Rust provider, but it has not been observed
  exchanging gossip messages with a JS-provider peer through
  `relay.sbvh.nl` in a deployed test.

## Why the JS default

Production runs `?provider=js` because the JS provider has been
observed cross-tab and through the deployed relay; the Rust provider
has not. Reliability > novelty. The Rust provider remains opt-in
behind the URL flag until 3b.3 verification lands.

## To reach "finished" for the Rust provider

1. Boot a browser with `?provider=rust` against `relay.sbvh.nl`.
2. Boot a second browser with the JS default.
3. Watch the event log: both should publish position broadcasts on
   `roam-positions/v1`; both should ingest the other's messages.
4. Repeat with both browsers on `?provider=rust`.
5. If all three combinations show round-trip gossip, the switch is
   verified and `default = ["rust-libp2p"]` can become the runtime
   default by flipping `PROVIDER` in `js-bridge.js`.

This is a verification step, not a code step. Until it's run, the
runtime default stays `js`.
