# roam/src/net — provider seam status

## Current architecture (0.3.2)

The `NetworkProvider` trait (`mod.rs`) is the seam. One concrete
implementation lives in this directory, plus a thin main-thread
bridge:

- `rust_libp2p.rs` — `RustLibp2pProvider`. Direct rust-libp2p in
  wasm32. Real `Swarm` with `websocket-websys` + `webrtc-websys`
  transports, gossipsub + identify + ping behaviours,
  `wasm_bindgen_futures::spawn_local` driver task. Runs inside the
  network web worker (`assets/src/net-worker.js`). On non-wasm32
  builds, a `new_stub` constructor exists so trait surface stays
  exercised by unit tests.
- `worker_bridge.rs` — `WorkerBridge`. Main-thread provider whose
  five callbacks `postMessage` commands to the worker and drain
  events the worker delivers via `onmessage`. Same trait, different
  transport.

## Field invariant — `Author` / `Forwarder`

`NetEvent::Message.from` is typed `Author` (newtype over `PeerId`).
The libp2p boundary in `rust_libp2p.rs::build_authored_message`
reads `gossipsub::Message.source` (the signed author), not
`propagation_source` (the immediate hop). The type forbids ever
passing a `Forwarder` into the slot. See the F2 test in `mod.rs`
for the falsifiable invariant.

## History

- 0.3.1 — TS Bun relay retired (replaced by `roam/relayers/`).
  Persistent IndexedDB identity end-to-end. Public SRE-style status
  page on `relay.sbvh.nl`.
- 0.3.2 — js-libp2p substrate retired. `?provider=` URL toggle
  removed. `JsLibp2pProvider` renamed to `WorkerBridge`. All
  `@libp2p/*` + `@chainsafe/*` + `libp2p` npm deps dropped.
