# roam — transport decisions

A running record of the substrate-level choices behind the production
network stack. Each entry: *what we considered, what we knew when we
decided, what we deferred, what would re-open the decision.*

---

## 2026-06-21 — WebTransport vs WebSocket (0.3.6)

### Context

0.3.6 verification surfaced an asymmetry: pickups propagate
end-to-end across two browser tabs (proven by
`tests/m6_via_relayer.rs`), but at the same time the user's tab saw
sustained `NoPeersSubscribedToTopic` on every `roam-positions/v1`
publish — collapse-counts ≥85/log-row over minutes. Same client
subscribe path, same relayer, same gossipsub config. The obvious
asymmetry between the two topics: positions publish at ~5 Hz,
pickups are sporadic.

The natural question: *should we move browser ↔ relayer from
WebSocket to WebTransport?*

### Mechanical comparison

| | WebSocket (current) | WebTransport |
|---|---|---|
| Transport layer | TCP + TLS | QUIC over UDP |
| Streams per connection | Single — TCP head-of-line blocking applies | Multiplexed independent streams |
| Datagram channel | No (all frames reliable + ordered) | Yes — unreliable + low-latency, the textbook fit for 5 Hz position broadcasts |
| Loss recovery | TCP retransmit blocks subsequent frames on the same connection | Per-stream + datagrams skip lost packets entirely |
| libp2p 0.56 support | `websocket-websys` (browser) / `websocket` (native) — both in production | `webtransport-websys` exists in the crate graph |
| Browser support | universal | landed in Firefox + Chromium some time ago; Safari uncertain — **must check `caniuse.com/webtransport` at decision time** |
| Relayer-side support | Already deployed via `libp2p-websocket` (soketto) | Would require adding `libp2p-webtransport` to the relayer + cert / Caddy / CloudFront path changes |

### What we knew at decision time

- `tests/m6_via_relayer.rs` proves the M6 pickup invariant over real
  gossipsub through the real relayer.
- `tests/positions_high_rate.rs` publishes the production
  position cadence (50 messages at 5 Hz) over the same real-wire
  harness and **passes** — zero `NoPeersSubscribedToTopic` from any
  publish, ≥ 90% delivery. **TCP HOL-blocking is not what's hitting
  the user's session**, since the native test is also TCP.
- The user's symptom is therefore specific to the wasm32 code path
  (`libp2p-websocket-websys` + the browser's runtime) or to a state
  of the deployed relayer at the time of the screenshot. Neither has
  been instrumented yet.

### Decision

**Diagnose the wasm path first. Defer the WebTransport switch.**

Switching transports based on the evidence we have would be
*fix-by-replacing*: the symptom might disappear because we sidestep
`websocket-websys`, but we wouldn't understand why the old path
failed. We'd inherit a different surface area (WebTransport server
config, cert handling, mesh formation timing, datagram-vs-stream
routing) without having narrowed the original bug.

WebTransport is genuinely the right substrate long-term for roam's
traffic shape (~5 Hz position broadcasts are a textbook datagram
fit). This is a deferral, not a rejection.

### What would re-open the decision

Any of:

1. Wasm-path diagnosis pins the bug in `libp2p-websocket-websys`
   itself (rather than in our wiring on top of it).
2. A browser-based integration test (playwright + actual dev server
   + relayer) reproduces the position-publish failure, and the same
   test against a WebTransport variant passes.
3. v0.4's cards-on-the-ground introduces additional high-rate
   topics (e.g. card-pickup contention broadcasts at game-tick
   cadence) where TCP head-of-line blocking would compound.
4. A non-trivial fraction of players join from connections with
   loss profiles that make TCP retransmit penalties unacceptable
   for position smoothness (verifiable from CloudWatch metrics on
   relayer publish latency vs client-perceived staleness).

### Verification gates for the eventual switch

Before WebTransport ships:

- Verify current browser support via `caniuse.com/webtransport` at
  decision time, not from training data or memory.
- Stand up a real-wire integration test analogous to
  `tests/positions_high_rate.rs` but using WebTransport on both
  ends; assert the same delivery threshold.
- Update the relayer to accept WebTransport in addition to
  WebSocket, with a deprecation window for the WS path.
- Update CloudFront / Caddy routing for HTTP/3 + UDP if required.
