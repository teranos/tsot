# laye — browser p2p sibling for game

Handoff doc. Read `game/CLAUDE.md`, `game/Architecture.md`, `game/imports.allow`, `game/src/remote_players.rs`, `game/src/net.rs`, and the `connectProxy` block in `game/web/src/main.ts` for full context.

## Background

**Roam had libp2p in the browser via wasm-bindgen.** Thousands of imports crossed the seam — no way to see what was happening between wasm and the browser. A leak grew that could not be found through the browser.

**Rave was built to reproduce the leak in isolation.** It couldn't. The seam itself was the problem — an opaque interface with thousands of surfaces cannot be inspected.

**Seer emerged as the external observer.** It reads what crosses a well-defined wasm↔browser boundary. Its ability to observe depends on the boundary being narrow enough to catalog.

**Game rewrites the boundary.** No wasm-bindgen in game.wasm. Every crossing is a hand-wired `#[link(wasm_import_module = "env")]` extern, and every one appears in `imports.allow`. `crates/seer-imports-check` runs on every build, diffs the wasm's import section against `imports.allow`, and fails on drift. This is enforced in CI.

**Initial architectural direction (rejected this session).** Relaye absorbs a WebSocket byte-pump role — game.wasm connects to `wss://relaye.sbvh.nl/ws/<topic>`, relaye reads/writes gossipsub on its behalf. The spec was `game/docs/relaye-game-gateway.md` (now deleted). This was rejected because it moves libp2p off the browser entirely — a decision game.wasm did not need to make. game.wasm's constraint is that libp2p must not be *in game.wasm*, not that it must not be *in the browser tab*.

**Current direction.** libp2p runs in the browser tab, in a sibling wasm module beside game.wasm. Both modules load in the same tab. game.wasm's env.* seam does not change. JS glue routes bytes between the sibling wasm and the env.* queue. Relaye reverts to being a plain libp2p peer in the mesh, not a gateway. Seer still observes game.wasm from outside; the sibling wasm is not its concern.

## What laye is asked to produce

**One shippable wasm bundle.** Loadable in the same tab as game.wasm via `<script>` or `WebAssembly.instantiate`. Uses wasm-bindgen freely — that's fine, this is not game.wasm.

**Internals.**
- A thin Bevy app consuming `bevy-libp2p` + `laye-me`.
- rust-libp2p with the wasm32 stack: `websocket-websys` transport, `noise` security, `yamux` multiplexer, `gossipsub` behaviour, `identify`, `ping`.
- Identity from `laye-me` (Ed25519 keypair, persisted to IndexedDB or similar; can be the same 32-byte key game already stores via `game_identity_load`/`game_identity_save`, or laye's own — coordinate).
- Subscribes to game's gossipsub topic. Today the topic is `rave-positions/v1` (inherited from rave); expected to rename to `game-positions/v1` at cutover. Coordinate with game.
- Dials relaye as bootstrap peer(s). `game should be multi-relay` (from game/README.md) — accept a list.
- No message parsing or validation. Payload bytes pass through verbatim.

**JS-callable surface.** Minimum:
- `init(bootstrap_config)` — start libp2p, connect to bootstrap peer(s), subscribe to topic(s). Config includes: bootstrap multiaddrs, topic name(s), identity bytes (or generate flag).
- `pending_bytes() -> u32` — how many receive bytes are queued.
- `recv_bytes(out_ptr, out_len) -> u32` — drain the queue into JS-owned memory.
- `publish(bytes_ptr, bytes_len)` — publish one gossipsub message on the configured topic.
- `self_peer_id() -> string` — Ed25519 pubkey / peer_id as a hex or bech32 string. Used by game.wasm as its self-identifier.

Signatures approximate; the actual JS-facing names are laye's call. What matters is the four-verb shape: **init, pending, drain, publish** (plus identity export).

**Wire contract.**
- **Payload** — one JSON `GamePosition {peer, x, y, z, at_ms}` per gossipsub message today. Verbatim pass-through; laye-p2p does not parse.
- **Framing** — one gossipsub message = one JS-visible unit. The JS glue re-frames into the length-prefixed queue game.wasm expects (`[u32 LE len][bytes]…` concatenated in `proxyRxBuf`).
- **Topic** — one for now (positions). Chat and catalog topics land later, same shape.

**Deployment.** Shipped alongside game.wasm from game.sbvh.nl (or wherever game deploys). CloudFront / S3 already handle multiple .wasm files fine.

## Invariants game.wasm relies on

These MUST NOT be violated:

- **`game/imports.allow` is authoritative.** Anything game.wasm imports must appear there. If laye's sibling wasm needs to expose new env.* imports *to game.wasm*, those must be added to imports.allow — but the current direction routes everything through the existing four (`game_peers_pending`, `game_peers_recv`, `game_self_publish`, `game_now_ms`). No new imports needed for the position path.
- **No wasm-bindgen in game.wasm.** Enforced by `imports.allow` + `seer-imports-check`. laye-p2p.wasm may use wasm-bindgen freely; game.wasm may not.
- **The four env.* import signatures don't change.** JS glue may re-point what fills the queue (from WebSocket to sibling-wasm output), but the wasm-side signatures stay identical.
- **Framing stays `[u32 LE len][bytes]…`** so game.wasm's `parse_frames` in `remote_players.rs` continues to work unchanged.
- **Seer observes game.wasm only.** laye-p2p.wasm is out of scope for seer. Its internals may be as busy as needed.

## Rollout order

1. Ship the wasm bundle + JS API. Verify standalone (two of it in two tabs see each other via gossipsub).
2. Wire it into game's `main.ts` in parallel with the existing WebSocket path. Toggle via querystring for verification.
3. Verify two browser tabs on game.sbvh.nl see each other with `?net=sibling-wasm` and still with `?net=relaye-ws`.
4. Cut over. Delete the `DEFAULT_PROXY_WS` / `connectProxy` block in main.ts.
5. Delete relaye's WS-gateway code (if it exists in `laye/crates/relaye/src/gateway.rs`). Relaye continues as plain libp2p peer.

## Out of scope for this handoff

- Signed message verification at relaye. Later.
- Chat topics. Same pattern, different topic. Trivial after positions works.
- Catalog subscription. Same pattern.
- Multi-topic subscription per client. Config extension.
- SharedArrayBuffer / zero-copy between the two wasms. Nice-to-have; requires COOP/COEP; not required.
