# relayers

**Goal: eliminate TypeScript from the network stack.**

`roam/relay/relay.ts` + `roam/relay/bun-ws-transport.ts` (~595
LOC) is the last TS in the path between the browser's rust-libp2p
substrate and the wire. Replace with a Rust binary at
`roam/relayers/`, behaviorally identical, same Lightsail box,
same systemd unit, same Secrets Manager identity, same DNS /
CloudFront layer. Browsers see no change.

The relayer is byte-opaque on gossipsub messages — it
re-broadcasts whatever bytes the bridge publishes. Wire-format
choices for the messages themselves are out of scope.

## Questions to answer before writing code

1. **Does rust-libp2p 0.56 server-side support every protocol the
   current relay runs**: gossipsub, identify, ping,
   circuit-relay-v2 (`maxReservations: 128`), websocket-tokio
   server transport?
2. **Can rust-libp2p decode the Ed25519 keypair already in the
   Secrets Manager secret `roam/relay/identity-dEQJoD`** — or do
   we need a format adapter (or accept a one-time PeerId
   rotation)?
3. **Does rust-libp2p's default gossipsub message-id function
   match js-libp2p's** `(peer-id, seqno)`? Mismatch = mesh
   dedupe breaks, every position broadcast amplifies.

Answer all three before step 2.

## Order

1. Answer Q1, Q2, Q3
2. Spike: rust-libp2p server locally, one browser connects,
   identify completes, ping does not abort
3. Port `relay.ts` + `bun-ws-transport.ts` to `roam/relayers/src/main.rs`
4. Local soak: 2 tabs, 30+ min, no disconnect, no message drops
5. Stage on Lightsail `:9002` alongside TS relay
6. Cut over: stop TS service, change `ExecStart` to Rust binary
7. Delete TS code after 1-week clean run

## Acceptance — when v1 replaces TS

- 2 tabs against deployed Rust relayer stay connected ≥1 hour
- Every gossipsub publish from one tab arrives at the other
- CloudWatch metrics unchanged in name, cadence, shape
- Restart → same PeerId (or one explicit rotation, never again)
- `cargo clippy` clean; no `unwrap()`, `unsafe`, `Box<dyn Error>`
  at the wire layer
- `connectionMonitor: { enabled: false } as any`-shape bugs
  structurally unrepresentable
- `roam/relay/` directory contains no TypeScript

## Status

- Decision: 2026-06-19, port to Rust
- Q1 / Q2 / Q3: not answered
- Spike: not started
- Port: not started

Update as each step lands. Details wrong → rewrite this file.
