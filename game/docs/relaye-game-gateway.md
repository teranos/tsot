# relaye — game.wasm gateway requirements

Handoff doc. Absorbs the `game-proxy` role into relaye so game.wasm
does not need a separate binary. Written from the game side; drop
into the relaye repo's docs/ as-is.

## Why relaye owns this

`game.wasm` is compiled without `wasm-bindgen`, so it cannot run
libp2p in the browser (no wasm-bindgen transports, crypto, timers).
It needs one native process on the other side of a WebSocket that
speaks libp2p to the mesh. Relaye already speaks libp2p to the mesh
for `rave.wasm`. Adding a WebSocket gateway inside relaye means one
process, one deploy, one TLS terminator, one systemd unit — instead
of shipping a second binary.

## What game.wasm expects

A WebSocket endpoint, no auth for MVP. Binary frames each way. Each
frame is one JSON payload byte-for-byte — the gateway does **not**
parse.

Payload shape (JSON; matches rave's `RavePosition` exactly):

```json
{ "peer": "abc123…", "x": 10.5, "y": 0.0, "z": -3.2, "at_ms": 1751888000000 }
```

Rx (mesh → browser): every gossipsub message on topic
`rave-positions/v1` → `ws.send(payload_bytes)` to every subscribed
client.

Tx (browser → mesh): every WS binary frame from a client →
`gossipsub.publish("rave-positions/v1", payload_bytes)`.

## Contract details

- **Topic**: `rave-positions/v1`. Do not rename until rave is
  decommed — game and rave players interop on the same topic during
  the transition.
- **Framing on WS**: one gossipsub message = one WS frame. No
  length prefix, no batching. Client-side handles multiple frames per
  tick by draining until the socket is empty.
- **Subscription lifecycle**: subscribe on connect, unsubscribe on
  disconnect. One client dropping does not affect others.
- **Identity**: gateway uses its own libp2p keypair (the relaye
  process identity). Browser identity lives in the payload's `peer`
  field — the gateway does not authenticate it.
- **Origin/CORS**: allow any. Public game.
- **Backpressure**: if the WS send buffer to a client fills up, drop
  the oldest queued frame — position updates are lossy by nature
  (10Hz stream, stale frames are worthless).

## Non-goals (do not add)

- No message signing/verification at the gateway. (Follow-up: browser
  signs with its 32-byte Ed25519 seed, verifiers check downstream.)
- No parsing or validation of payloads.
- No message ordering guarantees beyond what gossipsub provides.
- No per-client rate limit for MVP. Add if a client goes runaway.
- No REST endpoint, no admin, no metrics endpoint beyond whatever
  relaye already exposes.

## Endpoint

```
wss://relaye.sbvh.nl/ws/positions
```

Path is a suggestion — bikeshed to fit relaye's routing. The URL
gets plumbed into game.wasm via `game.sbvh.nl?proxy=wss://…`
querystring on the client side; game side does not care about the
path.

## Deployment

- Same process as existing relaye.
- Behind the same TLS terminator (nginx/caddy/whatever relaye uses).
- No new secrets. Uses existing libp2p keypair.

## Testing

Model on `crates/rave-positions-test` (which exercises rave's
libp2p side end-to-end):

- Spin relaye locally.
- Open two WS clients.
- ws1 sends `{peer:"a",x:1,y:0,z:0,at_ms:0}`.
- Assert ws2 receives the same bytes.
- Reverse direction.
- Publish through a native libp2p peer directly on
  `rave-positions/v1`; assert both WS clients receive it.

Round-trip parity with rave is the acceptance bar: a game.wasm
client and a rave.wasm client that both connect to the same relaye
must see each other move.

## Client-side already in place (for reference)

The game side (this repo, `game/src/remote_players.rs` +
`game/web/src/main.ts`) ships four env.* imports the wasm calls:

- `game_peers_pending()` — bytes queued from the WS
- `game_peers_recv(out_ptr, out_len)` — drain
- `game_self_publish(bytes_ptr, bytes_len)` — send one payload
- `game_now_ms()` — Date.now()

JS shim opens the WS from `?proxy=…`, buffers rx as length-prefixed
frames for the wasm to slice. Publish path is a straight
`ws.send(bytes)`. No changes needed on the client when this endpoint
ships.

## Open (out of scope for this handoff)

- Chat: `rave-chat/v1` will need the same treatment when game grows
  text chat. Trivial once positions works — same pattern, different
  topic.
- Signing.
- Multi-topic subscription per client (in case a client wants
  positions but not chat).
