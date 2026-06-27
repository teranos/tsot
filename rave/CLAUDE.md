rave — Bevy + libp2p rave party. Forked from `universe/` at its cells-stage prototype as starting code. The direction is peers in one shared room over libp2p.

Current (inherited from universe): WASD + drag, eat algae to grow, water particles repel from the player, camera follows. Press `` ` `` in-canvas for the diagnostic drawer (FPS, captured errors). This is scaffold to be replaced as the rave direction takes shape.

Workflow: edit, push to `rave` branch, CI builds + deploys to https://rave.sbvh.nl/. No local dev — Bevy compile cost lives in CI, not on this machine.

Bevy version is pinned in `Cargo.toml`. One tracker line in `BEVY.md` for the next-minor bump trigger.

**Errors are sacred** — panics + Bevy WARN/ERROR tracing events surface in the in-canvas drawer via `LogPlugin.custom_layer`. LogPlugin's console output is preserved (wrapped, not replaced). No silencing.

**Observability first** — the drawer is the in-canvas equivalent of devtools. If you can't see it, you don't know about it. Press `` ` `` to toggle.

Direction: peers connect over libp2p, end up in one shared room together. Mechanics, identity model, room model — all open.

## libp2p slice (in progress)

First multiplayer slice: two browsers running rave see each other as remote player cells, routed through `relay.sbvh.nl` (the relayer that already serves roam). Wire topic `rave-positions/v1` carries 10Hz position broadcasts as JSON `{peer, x, y, z, at_ms}`. Identity is an Ed25519 keypair persisted in IndexedDB (database `rave`, store `identity`); first visit mints it, every visit thereafter restores it.

The libp2p substrate is ported from roam's `rust-libp2p` provider (rust-libp2p 0.56.0 on `wasm32-unknown-unknown`, WebSocket-WebSys + noise + yamux + gossipsub + identify + ping + connection_limits, single-relay topology). Slice is intentionally small — no canonical/non-canonical class distinction (roam's invariant; rave's world model is different), no did:key surfacing yet. Extraction into a shared crate happens only after this slice runs and the API shape is shown by two real consumers.

Single-line git commits, no Claude attribution.
