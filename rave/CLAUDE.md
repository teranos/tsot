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

## Substrate

libp2p runs through `relay.sbvh.nl` (rust-libp2p 0.56.0 on
`wasm32-unknown-unknown`, WebSocket-WebSys + noise + yamux + gossipsub
+ identify + ping + connection_limits, single-relay topology — ported
from roam). Identity is an Ed25519 keypair in IndexedDB (db `rave`,
store `identity`). Module map + topic list are in `README.md`; don't
re-state them here.

The native integration test at `crates/rave-positions-test/` spins the
relayer binary on loopback + two libp2p clients and asserts a
`RavePosition` round-trips. Runs in CI before the wasm build. Replaces
the "open two browsers and look" manual check — any new gossipsub
topic gets a sibling test in the same crate.

## Module discipline

Add a new concern in a new module. Don't pile into `lib.rs` — it's the
orchestrator, not a feature dump.

## Open

- **Chatroom** — `rave-chat/v1` gossipsub topic, always-on translucent
  overlay, Enter-to-focus single-line input, scroll log. Only named
  pending slice.
- Humanoid avatars (Poly Pizza models on disk, not wired into
  `room::PlayerCell`).
- What "a rave" actually means past peers in a room with strobes.

Single-line git commits, no Claude attribution.
