rave — Bevy + libp2p rave party. Walkable 3D club: DJ booth + speakers on
the north wall, bar along the west, toilets along the east, garderobe +
entrance gap on the south, dancefloor in the middle with an overhead
truss carrying 6 colour-cycling spotlights + 4 corner strobes. Peers
walk through, see each other as spheres, identity persists per browser
via IndexedDB.

Press `` ` `` or `\` in-canvas for the diagnostic drawer (FPS, error
list, net stats, clock). Press `P` to copy a screenshot to clipboard.

Workflow: edit, push to `rave` branch, CI builds + deploys to
https://rave.sbvh.nl/. No local dev — Bevy compile cost lives in CI,
not on this machine.

Bevy version pinned in `Cargo.toml`. Open Bevy items in `BEVY.md`.

**Errors are sacred** — panics + Bevy WARN/ERROR tracing events + typed
`sacred_error::Error` values all surface in the in-canvas drawer + the
HTML overlay via the `observability` module. LogPlugin's console output
is preserved (wrapped, not replaced). No silencing.

**Observability first** — the drawer is the in-canvas equivalent of
devtools. If you can't see it, you don't know about it.

## libp2p slice — shipped

Two browsers loading rave see each other through `relay.sbvh.nl`. Wire
topic `rave-positions/v1` carries 10Hz position broadcasts as JSON
`{peer, x, y, z, at_ms}`. Identity is an Ed25519 keypair persisted in
IndexedDB (database `rave`, store `identity`); first visit mints it,
every visit thereafter restores it. The libp2p substrate is ported from
roam's `rust-libp2p` provider (rust-libp2p 0.56.0 on
`wasm32-unknown-unknown`, WebSocket-WebSys + noise + yamux + gossipsub +
identify + ping + connection_limits, single-relay topology).

The native integration test at `crates/rave-positions-test/` spins the
relayer binary on loopback + two native libp2p clients and asserts a
RavePosition round-trips. Runs in CI before the wasm build. Replaces
the "open two browsers and look" manual check.

## Module discipline

Add a new concern in a new module. Don't pile into `lib.rs` — it's the
orchestrator, not a feature dump. The current module map is in
`README.md`.

## Web layer

`rave/web/` is a bun + TypeScript project. Six small modules (overlay,
error-bridge, identity-bridge, screenshot, loading, main) plus
`bridges.d.ts` for the `window.__rave*` extern type declarations.
`bun build` produces `dist/main.js` which the Makefile content-hashes
into `main.<h>.js`. Browser sees static JS only — bun is build-time
only.

## Direction (still open)

Mechanics, chatroom UI, humanoid avatars (PolyPizza models are on
disk, not yet wired in), what "a rave" actually means past peers in a
room with strobes — all open. The shipped slice is the substrate; the
party is what you build on it.

Single-line git commits, no Claude attribution.
