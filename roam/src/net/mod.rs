//! Network seam — implementation-pluggable provider trait and the
//! data types that flow across it.
//!
//! The application talks to a `NetworkProvider` trait object; impls
//! live in sibling modules. The trait surface is deliberately narrow
//! — bytes in, bytes out, peer events — so the substrate is swappable
//! across language runtimes (js-libp2p, rust-libp2p, external process
//! speaking the wire envelope).
//!
//! ## Why this exists
//!
//! Today the Rust side knows nothing about networking; all of libp2p
//! — connections, messages, mesh state, peer table — lives in
//! `assets/src/js-bridge.js`. That makes "swap js-libp2p for
//! rust-libp2p" a migration of application logic across a language
//! boundary, not a substrate swap.
//!
//! A JS-side seam (provider interface in JS) only unlocks
//! "JS-with-different-backends"; everything still has to pass
//! through the browser's JS runtime as the meeting point. A Rust-
//! side seam (this file) lets any backend that can be wrapped as a
//! Rust trait impl plug in — including remote processes in any
//! language that speak the wire envelope.
//!
//! ## Roadmap
//!
//! ### Phase 1 — types only (done)
//!
//! Trait, types, wire-message enum, serde round-trip tests. No impl,
//! no behavior change.
//!
//! Files: `roam/src/net/mod.rs` (this), `roam/src/lib.rs` (module decl).
//!
//! ### Phase 2 — application logic in Rust, first provider impl (done)
//!
//! Application-layer network code lives in Rust. The bridge keeps
//! only provider-internal concerns (libp2p instance, redial driver,
//! raw-WS probes, relay-multiaddr fetch) plus the five-function shim
//! in `net-shim.js`. `Net` owns `Box<dyn NetworkProvider>` plus the
//! peer table; it's embedded in `World` as `Option<Net>` (attached
//! once libp2p init resolves on the JS side).
//!
//! Provider impl: `JsLibp2pProvider`. Construction takes five
//! `js_sys::Function` callbacks (publish, subscribe, unsubscribe,
//! self-peer-id, drain-events) rather than wasm-bindgen extern
//! imports, which avoids a separate JS module path that bun would
//! also have to know about. Functionally equivalent.
//!
//! Files that landed:
//! - `roam/src/net/mod.rs` — trait, types, wire-message enum.
//! - `roam/src/net/state.rs` — `Net`, `RemotePeer`, `tick`,
//!   `publish_position`, `subscribe_positions`, `peer_state_seq`.
//! - `roam/src/net/js_libp2p.rs` — `JsLibp2pProvider` over five JS
//!   callbacks; per-target cfg keeps the same struct compilable for
//!   native unit tests.
//! - `roam/src/world.rs` — `pub net: Option<Net>`.
//! - `roam/src/wasm_ffi.rs` — exports `roam_net_init`,
//!   `roam_net_tick`, `roam_net_publish_position`,
//!   `roam_net_peer_count`, `roam_net_peer_state_seq`. The render
//!   FFI populates peer markers from `Net.peers` before each draw.
//! - `roam/assets/src/net-shim.js` — five thin functions over
//!   `libp2p` / `pubsub`.
//! - `roam/assets/src/js-bridge.js` — bridge calls `roam_net_init`
//!   after libp2p is up, drives `roam_net_tick` per frame, replaces
//!   the position broadcast with `roam_net_publish_position`.
//!
//! Wire format on the pubsub topic stays JSON `{peer_id, x, y, z, f}`
//! for backward compatibility with pre-2b peers. `postcard` and the
//! `WireMsg` enum land alongside `RustLibp2pProvider` in phase 3 (when
//! we control both sides of the wire).
//!
//! ### Phase 3 — alternate impls (not started)
//!
//! Each is a swap at construction, decoupled from application code:
//!
//! - `RustLibp2pProvider` — direct rust-libp2p (`libp2p`,
//!   `libp2p-websocket-websys`, `libp2p-webrtc-websys`,
//!   `libp2p-gossipsub`). No JS in the network path; in-wasm crypto
//!   (no `SubtleCrypto` worker round-trip per sign). When we land
//!   this we also switch the wire to `postcard` since both ends are
//!   Rust.
//!
//! - `RemoteServerProvider` — Rust struct opening a WebSocket to an
//!   external process (Elixir, Go, …) speaking a serde-defined wire
//!   envelope. The remote process owns its own libp2p (or libcluster,
//!   or anything). The application doesn't know.
//!
//! ### What this seam unblocks
//!
//! - Lamport-timestamped pickup conflict resolution (v0.4) lives in
//!   Rust next to the world state, not across the FFI.
//! - Canonical/non-canonical identity routing (see CANONICAL.md)
//!   sits naturally at message-receive in `Net::tick`, in Rust,
//!   next to the state it mutates.
//! - One sacred-error pipeline: network errors flow through the
//!   same `error::push` path as everything else, no envelope
//!   marshaling at the JS↔Rust boundary.
//! - Deterministic replay: trace network events + game events on
//!   one timeline.
//!
//!
//! - **Phase 1** (types + trait): complete.
//! - **Phase 2a** (skeletons): complete — `state::Net`,
//!   `state::RemotePeer`, `js_libp2p::JsLibp2pProvider`.
//! - **Phase 2b** (outgoing through the seam): complete — the bridge's
//!   broadcast timer calls `roam_net_publish_position`, which goes
//!   `Net::publish_position` → `provider.publish` → `net-shim.js`
//!   → `pubsub.publish`. Wire format owned by Rust.
//! - **Phase 2c** (incoming through the seam): complete — incoming
//!   pubsub messages queue in `net-shim.js`, drain in
//!   `JsLibp2pProvider::poll_events`, route through `Net::tick` which
//!   decodes positions and updates the Rust-owned peer table.
//! - **Phase 2d** (delete JS-side peer table): complete — the
//!   bridge's `remotePeers` Map, `ingest` function, the legacy
//!   pubsub `message` listener that fed it, the BroadcastChannel
//!   fallback, and the `roam_set_peers` FFI are all gone. The
//!   renderer reads peers from `Net.peers` via `wasm_ffi`. The bridge
//!   reads peer count + change-counter through
//!   `roam_net_peer_count` / `roam_net_peer_state_seq`.
//! - **Phase 3** (alternate impls — `RustLibp2pProvider`,
//!   `RemoteServerProvider`): not started. They drop in via the same
//!   trait; application code in `Net` doesn't move.

pub mod js_libp2p;
pub mod state;

use serde::{Deserialize, Serialize};

/// libp2p-format peer identifier (the `12D3KooW…` string).
/// Newtype so the trait surface can't accidentally accept arbitrary strings.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(pub String);

/// A pubsub topic name. Newtype for the same reason as `PeerId`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Topic(pub String);

/// Failures the provider can surface. `reason` carries the provider-
/// internal message; the seam intentionally doesn't model every
/// libp2p failure as a typed variant — only the categories the
/// application reacts to differently. Routed through the sacred-error
/// pipeline at the call site (phase 2: `Net::tick` folds
/// `NetEvent::Error` into `error::push`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetError {
    PublishFailed { topic: Topic, reason: String },
    SubscribeFailed { topic: Topic, reason: String },
    NotConnected { reason: String },
    InvalidTopic { topic: Topic, reason: String },
    ProviderInternal { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NetEvent {
    PeerUp { peer: PeerId, addrs: Vec<String> },
    PeerDown { peer: PeerId, reason: String },
    Message {
        topic: Topic,
        from: PeerId,
        bytes: Vec<u8>,
        at_ms: u64,
    },
    SubscriptionChange { topic: Topic, peer: PeerId, joined: bool },
    Error(NetError),
}

/// What flows on a pubsub topic. Both sides serialize/deserialize
/// against this type — same Rust definition on every node. New
/// message shapes are added as variants; the wire is one enum, not a
/// free-form schema.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WireMsg {
    Position { x: f32, y: f32, facing: u8, at_ms: u64 },
}

/// The seam. Phase 1 declares it; phase 2 supplies the first impl
/// (`JsLibp2pProvider`). Future impls (`RustLibp2pProvider`,
/// `RemoteServerProvider`) plug in via the same trait — application
/// code holds `Box<dyn NetworkProvider>`, never the concrete type.
///
/// Sync surface deliberately: matches the existing tick-based engine
/// (`World::step`). Providers buffer asynchronously internally and
/// expose accumulated events through `poll_events`. `publish` is
/// effectively fire-and-forget; failures surface via `NetEvent::Error`
/// on the next poll, not via the immediate return value (which only
/// catches synchronous provider-internal failures like serialization).
pub trait NetworkProvider {
    fn identity(&self) -> PeerId;
    fn publish(&mut self, topic: &Topic, bytes: &[u8]) -> Result<(), NetError>;
    fn subscribe(&mut self, topic: &Topic) -> Result<(), NetError>;
    fn unsubscribe(&mut self, topic: &Topic) -> Result<(), NetError>;
    fn poll_events(&mut self) -> Vec<NetEvent>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    fn round_trip<T>(value: T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(&value).expect("serialize");
        let parsed: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, parsed);
    }

    #[test]
    fn peer_id_round_trips() {
        round_trip(PeerId("12D3KooWXYZ".to_string()));
    }

    #[test]
    fn topic_round_trips() {
        round_trip(Topic("roam-positions/v1".to_string()));
    }

    #[test]
    fn net_error_variants_round_trip() {
        round_trip(NetError::PublishFailed {
            topic: Topic("t".into()),
            reason: "queue full".into(),
        });
        round_trip(NetError::SubscribeFailed {
            topic: Topic("t".into()),
            reason: "transport down".into(),
        });
        round_trip(NetError::NotConnected {
            reason: "no mesh peers".into(),
        });
        round_trip(NetError::InvalidTopic {
            topic: Topic("".into()),
            reason: "empty topic name".into(),
        });
        round_trip(NetError::ProviderInternal {
            reason: "wasm-bindgen panic".into(),
        });
    }

    #[test]
    fn net_event_variants_round_trip() {
        round_trip(NetEvent::PeerUp {
            peer: PeerId("p".into()),
            addrs: vec!["/dns4/x/tcp/443/wss".into()],
        });
        round_trip(NetEvent::PeerDown {
            peer: PeerId("p".into()),
            reason: "timeout".into(),
        });
        round_trip(NetEvent::Message {
            topic: Topic("t".into()),
            from: PeerId("p".into()),
            bytes: vec![1, 2, 3],
            at_ms: 1_700_000_000_000,
        });
        round_trip(NetEvent::SubscriptionChange {
            topic: Topic("t".into()),
            peer: PeerId("p".into()),
            joined: true,
        });
        round_trip(NetEvent::Error(NetError::NotConnected {
            reason: "no mesh peers".into(),
        }));
    }

    #[test]
    fn wire_msg_position_round_trips() {
        round_trip(WireMsg::Position {
            x: 1234.5,
            y: -678.25,
            facing: 2,
            at_ms: 1_700_000_000_000,
        });
    }

    #[test]
    fn provider_trait_is_object_safe() {
        // Compile-time check: a Box<dyn NetworkProvider> is the
        // application-level holding shape, so the trait must remain
        // object-safe across future revisions.
        fn _accepts(_p: Box<dyn NetworkProvider>) {}
    }
}
