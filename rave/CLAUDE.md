**This is a persistent universe.** Procedurally seeded, peer-editable,
cycling through heat-death and rebirth — Ouroboros. Germ-line identity
persists through every cycle. The rave is one thing happening in it
right now; it isn't the point.

Bevy + libp2p on `wasm32-unknown-unknown`, deployed at
https://rave.sbvh.nl/ via CI on push to `master`. Edit, push, watch
CI. No local dev — Bevy compile cost lives in CI, not on this
machine.

Press `` ` `` or `\` in-canvas (or the top-right `≡` button on mobile)
for the diagnostic drawer (FPS, errors, net stats, clock). Press `P`
for a screenshot.

**Errors are sacred** — panics + Bevy WARN/ERROR tracing + typed
`sacred_error::Error` all surface in the in-canvas drawer + the HTML
overlay via the `observability` module. No silencing.

**Observability first** — the drawer is the in-canvas equivalent of
devtools. If you can't see it, you don't know about it.

## Substrate

libp2p runs through `relay.sbvh.nl` via `bevy-libp2p` from the laye
workspace (single-relay topology — ported from roam). Identity is an
Ed25519 keypair in IndexedDB (db `rave`, store `identity`). Two
topics on the wire: `rave-positions/v1` (10Hz XYZ) and
`rave-chat/v1` (lines of text, Enter to send). Module map is in
`README.md`; don't re-state it here.

The native integration test crate at `crates/rave-positions-test/`
spins the relayer binary on loopback + two libp2p clients and asserts
each wire topic round-trips through it (one test file per topic).
Runs in CI before the wasm build. Replaces the "open two browsers and
look" manual check — any new gossipsub topic gets a sibling test in
the same crate.

## Module discipline

Add a new concern in a new module. Don't pile into `lib.rs` — it's the
orchestrator, not a feature dump.

## The universe

Procedurally generated from a deterministic seed (Wang-hash style,
like roam) + a per-peer-writable delta layer on top. First version:
unlimited write access for everyone. Edits propagate via a future
`rave-universe/v1` gossipsub topic; persistence path TBD between
relayer-hosted snapshot, per-peer IndexedDB + re-share on join, and
libp2p DHT.

The universe lives ~60 minutes before heat-death, then **Ouroboros**
restarts it. The last 10 seconds of the dying universe are the first
10 of the next. Germ-line identity persists across the cycle via the
player's keypair. Endgame: retain as much entropy as possible against
the universal flatten.

A rave happens in it. So does everything else, eventually.

## Open

- Humanoid avatars (Poly Pizza models on disk, not wired into
  `room::PlayerCell`).
- Persistence path for universe deltas.
- What carries through Ouroboros — the meta-game.

Single-line git commits, no Claude attribution.
