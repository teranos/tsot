//! Application-layer network state owned by Rust.
//!
//! `Net` holds the provider (behind the trait so substrate swaps are
//! a construction-time choice) and the peer table that the renderer
//! reads from. Embedded in `World` in phase 2b; phase 2a establishes
//! the shape only — `Net::tick` is a placeholder and no code calls
//! it yet.
//!
//! The peer table replaces the JS-side `remotePeers` Map that lives
//! in `assets/src/js-bridge.js` today. Once `Net` is wired into the
//! frame loop (2c → 2d), the JS-side table is deleted along with
//! `roam_set_peers`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::net::{Author, NetError, NetEvent, NetworkProvider, PeerId, Topic};

/// Topic where roam broadcasts player positions. Same string used by
/// every node — gossipsub treats this as a flat namespace.
pub const POSITIONS_TOPIC: &str = "roam-positions/v1";

/// Topic where canonical-class flower pickups propagate. Each message
/// claims a single `(x, y)` tile in canonical world state; identified
/// peers consume the claim into their local `canonical_picked` set,
/// which the renderer reads to hide the flower from every canonical
/// player's view. Non-canonical players do not publish here — their
/// pickups stay in `Player.picked` (sandbox-only).
pub const PICKUPS_TOPIC: &str = "roam-pickups/v1";

/// Topic where the relayer publishes its card catalog. The relayer is
/// the authority for which cards exist in its world — different
/// relayer = different catalog = different cards on the ground. Clients
/// subscribe; the relayer republishes periodically (no request/response
/// protocol). See `roam/relayers/src/main.rs::CATALOG_TOPIC`.
pub const CATALOG_TOPIC: &str = "roam-catalog/v1";

/// Every gossipsub topic this node subscribes to. The JS bridge reads
/// this list via `roam_subscribed_topics_json` on worker-ready and
/// dispatches one `subscribe` command per topic to the network worker.
/// Single source of truth: adding a topic is a one-line edit here,
/// JS never carries topic strings.
pub const ALL_TOPICS: &[&str] = &[POSITIONS_TOPIC, PICKUPS_TOPIC, CATALOG_TOPIC];

/// One remote peer's last-known state. Rendered as a marker on the
/// world canvas in phase 2d. Stale entries (peers we haven't heard
/// from in `PEER_TIMEOUT_MS`) are pruned in `Net::tick` to avoid
/// rendering ghosts.
#[derive(Clone, Debug, PartialEq)]
pub struct RemotePeer {
    /// The author of the position messages we've been tracking.
    /// Typed `Author` (not bare `PeerId`) so the table is structurally
    /// keyable only by signed authorship — never by a forwarder hop.
    pub peer_id: Author,
    pub x: f32,
    pub y: f32,
    pub facing: u8,
    pub last_seen_ms: u64,
}

/// Stale-peer threshold in milliseconds. Mirrors `PEER_TIMEOUT_MS` in
/// `js-bridge.js`; consolidated here when the JS-side table is
/// deleted in 2d.
pub const PEER_TIMEOUT_MS: u64 = 2000;

/// Application-layer network state. Holds the provider behind the
/// trait so swapping substrate (`JsLibp2pProvider`,
/// `RustLibp2pProvider`, `RemoteServerProvider`, …) is a
/// construction-time choice, not an application-code change.
pub struct Net {
    provider: Box<dyn NetworkProvider>,
    peers: BTreeMap<Author, RemotePeer>,
    /// Monotonic counter that bumps whenever the peer table changes
    /// — insert, remove, or position update. The render bridge reads
    /// this to feed the dirty-flag fingerprint so peers moving on
    /// screen actually triggers a repaint when the local player is
    /// still. `wrapping_add` so 32-bit u32 dirty-flag stays cheap to
    /// read across the FFI without BigInt-marshaling at the boundary.
    peer_state_seq: u32,
    /// Canonical-class pickup claims received via gossipsub, waiting
    /// for the FFI's per-tick drain to apply them into
    /// `World.canonical_picked`. Per-tick batching avoids interleaving
    /// network ingress with frame state in mid-tick, which would race
    /// the renderer reading `canonical_picked` during viewport build.
    pending_canonical_pickups: Vec<(i32, i32)>,
    /// Most recent catalog received from the relayer, waiting for the
    /// FFI's per-tick drain to install it into `World.catalog`. Some =
    /// freshly received, drained on next tick; None = nothing new.
    /// The relayer republishes periodically, so a missed message just
    /// means the next interval picks up. See `CATALOG_TOPIC`.
    pending_catalog: Option<Vec<crate::catalog::CatalogEntry>>,
}

impl Net {
    pub fn new(provider: Box<dyn NetworkProvider>) -> Self {
        Self {
            provider,
            peers: BTreeMap::new(),
            peer_state_seq: 0,
            pending_canonical_pickups: Vec::new(),
            pending_catalog: None,
        }
    }

    /// Read the peer-table change counter (see field doc).
    pub fn peer_state_seq(&self) -> u32 {
        self.peer_state_seq
    }

    /// Read-only iteration over the peer table. Used by the renderer
    /// in phase 2d to build the marker instance buffer; today (2a)
    /// no caller exists.
    pub fn peers(&self) -> impl Iterator<Item = &RemotePeer> {
        self.peers.values()
    }

    /// Drain provider events, update the peer table from incoming
    /// position messages, prune stale peers. Called once per frame
    /// from `roam_net_tick` in the FFI.
    pub fn tick(&mut self, now_ms: u64) {
        let events = self.provider.poll_events();
        for ev in events {
            match ev {
                NetEvent::Message {
                    topic,
                    from,
                    bytes,
                    at_ms,
                } => {
                    // Per-message trace removed in 0.3.6: at ~10
                    // events/sec/peer it bloated localStorage to quota
                    // exceeded and made the tab unusable. The F2-era
                    // cross-substrate-parity question it answered is
                    // settled — positions clearly flow. If a future
                    // debug need arises, add a rate-limited emit gated
                    // on a build flag rather than the per-message hot
                    // path.
                    if topic.0 == PICKUPS_TOPIC {
                        match serde_json::from_slice::<PickupWireIn>(&bytes) {
                            Ok(pick) => {
                                crate::perf::PICKUP_RECEIVED
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                self.pending_canonical_pickups.push((pick.x, pick.y));
                            }
                            Err(e) => {
                                #[cfg(target_arch = "wasm32")]
                                crate::error::emit(
                                    crate::error::Severity::Warn,
                                    "roam::net::Net::tick",
                                    "pickup decode failed",
                                    format!("from={from:?} reason={e}"),
                                );
                                #[cfg(not(target_arch = "wasm32"))]
                                let _ = e;
                            }
                        }
                        continue;
                    }
                    if topic.0 == CATALOG_TOPIC {
                        let raw = match std::str::from_utf8(&bytes) {
                            Ok(s) => s,
                            Err(e) => {
                                #[cfg(target_arch = "wasm32")]
                                crate::error::emit(
                                    crate::error::Severity::Warn,
                                    "roam::net::Net::tick",
                                    "catalog payload not utf-8",
                                    format!("from={from:?} reason={e}"),
                                );
                                #[cfg(not(target_arch = "wasm32"))]
                                let _ = (e, &from);
                                continue;
                            }
                        };
                        match crate::catalog::parse_catalog_json(raw) {
                            Ok(entries) => {
                                self.pending_catalog = Some(entries);
                            }
                            Err(why) => {
                                #[cfg(target_arch = "wasm32")]
                                crate::error::emit(
                                    crate::error::Severity::Warn,
                                    "roam::net::Net::tick",
                                    "catalog decode failed",
                                    format!("from={from:?} reason={why}"),
                                );
                                #[cfg(not(target_arch = "wasm32"))]
                                let _ = (why, &from);
                            }
                        }
                        continue;
                    }
                    if topic.0 != POSITIONS_TOPIC {
                        continue;
                    }
                    match serde_json::from_slice::<PositionWireIn>(&bytes) {
                        Ok(pos) => {
                            self.peers.insert(
                                from.clone(),
                                RemotePeer {
                                    peer_id: from,
                                    x: pos.x,
                                    y: pos.y,
                                    facing: pos.f,
                                    last_seen_ms: at_ms,
                                },
                            );
                            self.peer_state_seq = self.peer_state_seq.wrapping_add(1);
                        }
                        Err(e) => {
                            #[cfg(target_arch = "wasm32")]
                            crate::error::emit(
                                crate::error::Severity::Warn,
                                "roam::net::Net::tick",
                                "position decode failed",
                                format!("from={from:?} reason={e}"),
                            );
                            #[cfg(not(target_arch = "wasm32"))]
                            let _ = e;
                        }
                    }
                }
                NetEvent::PeerDown { peer, reason } => {
                    crate::trace::emit(crate::trace::TraceEvent::Note {
                        tag: "net::peer_down",
                        msg: format!("peer={} reason={}", peer.0, reason),
                    });
                    // PeerDown.peer is the immediate connection peer
                    // (a `Forwarder` semantically — in star topology
                    // always the relay), NOT the author. The peer
                    // table is `Author`-keyed; F2's whole point is
                    // that these are different identities and cannot
                    // be conflated. Stale authors get evicted by the
                    // PEER_TIMEOUT_MS pruner below, not here.
                }
                NetEvent::PeerUp { peer, addrs } => {
                    crate::trace::emit(crate::trace::TraceEvent::Note {
                        tag: "net::peer_up",
                        msg: format!("peer={} addrs={}", peer.0, addrs.join(",")),
                    });
                }
                NetEvent::SubscriptionChange { .. } => {}
                NetEvent::Error(err) => {
                    // Increment the per-topic delivery_err counter
                    // when this is an async PublishFailed — this is
                    // the truthful "publish didn't make it" measure
                    // that `publish_pickup`/`publish_position`'s
                    // sync-Ok counter alone couldn't see. The total
                    // delivered ≈ attempted − sync_err − delivery_err.
                    if let NetError::PublishFailed { topic, .. } = &err {
                        if topic.0 == PICKUPS_TOPIC {
                            crate::perf::PICKUP_PUBLISH_DELIVERY_ERR
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        } else if topic.0 == POSITIONS_TOPIC {
                            crate::perf::POSITION_PUBLISH_DELIVERY_ERR
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    // Stable-format trace so the JS-side fingerprint
                    // collapse (tag-based, `js-bridge.js:354`) merges
                    // repeats into one row with `×N`. Once `net::recv`
                    // stops interleaving (silenced above this block),
                    // a flood of `NoPeersSubscribedToTopic` collapses
                    // to one visible line — visibility preserved, log
                    // pressure relieved.
                    crate::trace::emit(crate::trace::TraceEvent::Note {
                        tag: "net::provider_error",
                        msg: format!("{err:?}"),
                    });
                }
            }
        }
        // Prune stale peers (haven't seen them in PEER_TIMEOUT_MS).
        let before = self.peers.len();
        self.peers
            .retain(|_, p| now_ms.saturating_sub(p.last_seen_ms) < PEER_TIMEOUT_MS);
        if self.peers.len() != before {
            self.peer_state_seq = self.peer_state_seq.wrapping_add(1);
        }
    }

    /// Inject identity for tests + the eventual `roam_state_json` path
    /// — read-only access through the trait surface keeps the seam
    /// closed.
    pub fn identity(&self) -> PeerId {
        self.provider.identity()
    }

    /// Broadcast the local player's position on the canonical
    /// positions topic.
    ///
    /// Wire format is the existing JSON envelope (`peer_id`, `x`, `y`,
    /// `z`, `f`) so this is wire-compatible with peers running the
    /// pre-2b js-bridge.js code. A postcard wire envelope lands as a
    /// separate slice once incoming messages are routed through the
    /// seam too.
    //
    // IDENTITY MENU (roam/docs/IDENTITY.md):
    //   S7 — sign the broadcast with the identity key; verify on receive.
    //        Wire-format change is part of this slice.
    pub fn publish_position(
        &mut self,
        x: f32,
        y: f32,
        z: i32,
        facing: u8,
    ) -> Result<(), NetError> {
        #[derive(Serialize)]
        struct PositionWire<'a> {
            peer_id: &'a str,
            x: f32,
            y: f32,
            z: i32,
            f: u8,
        }
        let identity = self.provider.identity();
        let msg = PositionWire {
            peer_id: &identity.0,
            x,
            y,
            z,
            f: facing,
        };
        crate::perf::POSITION_PUBLISH_ATTEMPTED
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes = serde_json::to_vec(&msg).map_err(|e| {
            crate::perf::POSITION_PUBLISH_ERR
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            NetError::ProviderInternal {
                reason: format!("position encode failed: {e}"),
            }
        })?;
        let result = self
            .provider
            .publish(&Topic(POSITIONS_TOPIC.to_string()), &bytes);
        match &result {
            Ok(_) => {
                crate::perf::POSITION_PUBLISH_OK
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(_) => {
                crate::perf::POSITION_PUBLISH_ERR
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        result
    }

    /// Subscribe to the canonical positions topic. Called once from
    /// the FFI's `roam_net_init`.
    pub fn subscribe_positions(&mut self) -> Result<(), NetError> {
        self.provider.subscribe(&Topic(POSITIONS_TOPIC.to_string()))
    }

    /// M6 — broadcast a canonical flower-pickup claim. Called from
    /// `World::try_pickup`'s Canonical branch right after the local
    /// state mutation. Wire is the smallest viable JSON envelope
    /// (`{x, y}`); the author is the libp2p envelope's signed source,
    /// recoverable by any identified peer via `Author::did_key` per
    /// M5 — so the payload doesn't carry the claim of identity
    /// (libp2p already verified it).
    pub fn publish_pickup(&mut self, x: i32, y: i32) -> Result<(), NetError> {
        #[derive(Serialize)]
        struct PickupWire {
            x: i32,
            y: i32,
        }
        crate::perf::PICKUP_PUBLISH_ATTEMPTED
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let bytes = serde_json::to_vec(&PickupWire { x, y }).map_err(|e| {
            crate::perf::PICKUP_PUBLISH_ERR
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            NetError::ProviderInternal {
                reason: format!("pickup encode failed: {e}"),
            }
        })?;
        let result = self
            .provider
            .publish(&Topic(PICKUPS_TOPIC.to_string()), &bytes);
        match &result {
            Ok(_) => {
                crate::perf::PICKUP_PUBLISH_OK
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(_) => {
                crate::perf::PICKUP_PUBLISH_ERR
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        result
    }

    /// M6 — subscribe to the canonical pickups topic. Called once from
    /// the FFI's `roam_net_init` alongside `subscribe_positions`.
    pub fn subscribe_pickups(&mut self) -> Result<(), NetError> {
        self.provider.subscribe(&Topic(PICKUPS_TOPIC.to_string()))
    }

    /// Subscribe to the relayer's catalog topic. Required before the
    /// relayer's republished catalog can land in `pending_catalog`.
    pub fn subscribe_catalog(&mut self) -> Result<(), NetError> {
        self.provider.subscribe(&Topic(CATALOG_TOPIC.to_string()))
    }

    /// Take the most recently received catalog (or None). FFI calls
    /// this each tick; if Some, applies into `World.catalog`.
    pub fn take_pending_catalog(&mut self) -> Option<Vec<crate::catalog::CatalogEntry>> {
        self.pending_catalog.take()
    }

    /// M6 — drain the queue of canonical-class pickup claims received
    /// since the last call. The FFI applies each `(x, y)` to
    /// `World.canonical_picked` so the renderer hides the flower from
    /// every identified peer's view. Drain-on-read: a second call
    /// without new ingress returns empty.
    pub fn drain_pending_canonical_pickups(&mut self) -> Vec<(i32, i32)> {
        std::mem::take(&mut self.pending_canonical_pickups)
    }
}

/// Wire shape for incoming position messages. Matches the envelope
/// the bridge (and any pre-2b peers) put on the wire — `peer_id` and
/// `z` flow on the wire but we don't store them here (peer authorship
/// comes from the gossipsub `from` field; `z` isn't used by the
/// renderer yet). Serde ignores fields not declared.
#[derive(Deserialize)]
struct PositionWireIn {
    x: f32,
    y: f32,
    f: u8,
}

/// Wire shape for incoming canonical pickups. Matches the outbound
/// `PickupWire` exactly — same struct on both sides of the seam.
#[derive(Deserialize)]
struct PickupWireIn {
    x: i32,
    y: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{NetError, NetEvent, NetworkProvider, Topic};

    /// Minimal provider for unit tests — records calls so we can
    /// assert the seam plumbing without dragging in libp2p.
    struct StubProvider {
        id: PeerId,
        publishes: Vec<(Topic, Vec<u8>)>,
        subs: Vec<Topic>,
        unsubs: Vec<Topic>,
        events: Vec<NetEvent>,
    }

    impl StubProvider {
        fn new(id: &str) -> Self {
            Self {
                id: PeerId(id.into()),
                publishes: Vec::new(),
                subs: Vec::new(),
                unsubs: Vec::new(),
                events: Vec::new(),
            }
        }
    }

    impl NetworkProvider for StubProvider {
        fn identity(&self) -> PeerId {
            self.id.clone()
        }
        fn publish(&mut self, topic: &Topic, bytes: &[u8]) -> Result<(), NetError> {
            self.publishes.push((topic.clone(), bytes.to_vec()));
            Ok(())
        }
        fn subscribe(&mut self, topic: &Topic) -> Result<(), NetError> {
            self.subs.push(topic.clone());
            Ok(())
        }
        fn unsubscribe(&mut self, topic: &Topic) -> Result<(), NetError> {
            self.unsubs.push(topic.clone());
            Ok(())
        }
        fn poll_events(&mut self) -> Vec<NetEvent> {
            std::mem::take(&mut self.events)
        }
    }

    #[test]
    fn net_constructs_with_boxed_provider() {
        let net = Net::new(Box::new(StubProvider::new("12D3KooWself")));
        assert_eq!(net.identity().0, "12D3KooWself");
        assert_eq!(net.peers().count(), 0);
    }

    #[test]
    fn net_tick_with_no_events_leaves_peer_table_empty() {
        let mut net = Net::new(Box::new(StubProvider::new("12D3KooWself")));
        net.tick(1_700_000_000_000);
        assert_eq!(net.peers().count(), 0);
    }

    fn position_bytes(x: f32, y: f32, facing: u8) -> Vec<u8> {
        serde_json::json!({
            "peer_id": "ignored-the-from-field-is-authoritative",
            "x": x,
            "y": y,
            "f": facing,
            "z": 0,
        })
        .to_string()
        .into_bytes()
    }

    fn position_event(author_id: &str, x: f32, y: f32) -> NetEvent {
        NetEvent::Message {
            topic: Topic(POSITIONS_TOPIC.to_string()),
            from: Author(PeerId(author_id.to_string())),
            bytes: position_bytes(x, y, 0),
            at_ms: 1_700_000_000_000,
        }
    }

    /// Three authored position messages → three distinct peer-table
    /// entries. The placeholder test this replaces only asserted
    /// `peers().count() == 0` after a no-op tick — which is why F2
    /// (`propagation_source` masquerading as `from`) shipped without
    /// any state-level test catching it: every author would have
    /// collapsed to one peer in the table and nothing here would have
    /// noticed.
    ///
    /// Falsifiable: if Net::tick ever conflates authors (e.g. someone
    /// re-keys the BTreeMap on something other than `Author`, or the
    /// insert lookup uses the wrong field), this test trips with
    /// `assert_eq! left != right`. Mutation-verified by hand: replace
    /// `self.peers.insert(from.clone(), ...)` with
    /// `self.peers.insert(self.peers.keys().next().cloned().unwrap_or(from.clone()), ...)`
    /// → all three messages collapse to one entry → this assert
    /// fails.
    #[test]
    fn net_tick_records_three_distinct_authors_as_three_peers() {
        let mut provider = StubProvider::new("12D3KooWself");
        provider.events.push(position_event("12D3KooWAuthorA", 1.0, 2.0));
        provider.events.push(position_event("12D3KooWAuthorB", 3.0, 4.0));
        provider.events.push(position_event("12D3KooWAuthorC", 5.0, 6.0));

        let mut net = Net::new(Box::new(provider));
        net.tick(1_700_000_000_000);

        assert_eq!(
            net.peers().count(),
            3,
            "three distinct Author keys must produce three peer-table entries"
        );

        let ids: std::collections::BTreeSet<String> =
            net.peers().map(|p| p.peer_id.0 .0.clone()).collect();
        assert!(ids.contains("12D3KooWAuthorA"));
        assert!(ids.contains("12D3KooWAuthorB"));
        assert!(ids.contains("12D3KooWAuthorC"));
    }

    fn pickup_event(author_id: &str, x: i32, y: i32) -> NetEvent {
        let bytes = serde_json::json!({ "x": x, "y": y }).to_string().into_bytes();
        NetEvent::Message {
            topic: Topic(PICKUPS_TOPIC.to_string()),
            from: Author(PeerId(author_id.to_string())),
            bytes,
            at_ms: 1_700_000_000_000,
        }
    }

    /// M6 — incoming canonical pickup messages queue into Net so the
    /// FFI can drain them and apply to `World.canonical_picked` once
    /// per tick. The pickup topic and the positions topic flow through
    /// the same `NetEvent::Message` stream; Net.tick discriminates on
    /// topic and routes accordingly. Falsifies the regression where
    /// pickups silently land in the peer table (treated as position
    /// updates) or get discarded with no queue access.
    #[test]
    fn net_tick_queues_pickup_messages_for_application() {
        let mut provider = StubProvider::new("12D3KooWself");
        provider.events.push(pickup_event("12D3KooWPeerA", 11, 22));
        provider.events.push(pickup_event("12D3KooWPeerB", -5, 6));
        let mut net = Net::new(Box::new(provider));
        net.tick(1_700_000_000_000);
        let drained = net.drain_pending_canonical_pickups();
        assert_eq!(drained.len(), 2);
        assert!(drained.contains(&(11, 22)));
        assert!(drained.contains(&(-5, 6)));
    }

    /// M6 — draining the queue empties it. Falsifies the regression
    /// where pickups stay in the queue across calls (would cause the
    /// same canonical pickup to be re-applied every tick, exponential
    /// repeats in the canonical_picked set's churn metrics).
    #[test]
    fn drain_pending_canonical_pickups_empties_the_queue() {
        let mut provider = StubProvider::new("12D3KooWself");
        provider.events.push(pickup_event("12D3KooWPeerA", 1, 1));
        let mut net = Net::new(Box::new(provider));
        net.tick(1_700_000_000_000);
        assert_eq!(net.drain_pending_canonical_pickups().len(), 1);
        assert!(net.drain_pending_canonical_pickups().is_empty(), "second drain must be empty");
    }

    /// Re-broadcast of a position from the same author updates the
    /// existing peer entry rather than inserting a duplicate. This
    /// is the "I moved, here's my new (x,y)" path; the table must
    /// stay keyed by Author and the row's x/y must reflect the
    /// latest message.
    #[test]
    fn net_tick_reauthored_message_updates_position_no_duplicate() {
        let mut provider = StubProvider::new("12D3KooWself");
        provider.events.push(position_event("12D3KooWAuthorA", 1.0, 2.0));
        provider.events.push(position_event("12D3KooWAuthorA", 7.5, 8.5));

        let mut net = Net::new(Box::new(provider));
        net.tick(1_700_000_000_000);

        let peers: Vec<&RemotePeer> = net.peers().collect();
        assert_eq!(peers.len(), 1, "same author across two messages → one peer entry");
        assert!((peers[0].x - 7.5).abs() < 1e-6);
        assert!((peers[0].y - 8.5).abs() < 1e-6);
    }

    /// `ALL_TOPICS` is the canonical list the JS bridge reads via
    /// `roam_subscribed_topics_json`. It must contain every topic
    /// the application publishes to or expects to receive on. A new
    /// topic that's added to the publish/subscribe paths but missing
    /// here will silently fail to deliver across peers (gossipsub
    /// won't announce the subscribe → mesh empty for the topic →
    /// PublishFailed flood, no propagation). This test pins the
    /// invariant that every topic the code uses lives in ALL_TOPICS.
    #[test]
    fn all_topics_contains_every_topic_the_code_uses() {
        assert!(ALL_TOPICS.contains(&POSITIONS_TOPIC));
        assert!(ALL_TOPICS.contains(&PICKUPS_TOPIC));
    }

    /// No duplicate topic entries — duplicates would double-subscribe
    /// the same topic which is harmless but signals a list-hygiene
    /// regression that's cheap to catch here.
    #[test]
    fn all_topics_has_no_duplicates() {
        for (i, a) in ALL_TOPICS.iter().enumerate() {
            for b in &ALL_TOPICS[i + 1..] {
                assert_ne!(a, b, "duplicate topic in ALL_TOPICS: {a}");
            }
        }
    }

    // Mock-based M6 propagation test removed: a fake mesh skips the
    // gossipsub protocol — the layer the production bug (missing
    // PICKUPS_TOPIC subscribe announcement) actually lives at. The
    // real-wire equivalent runs in `roam/tests/m6_via_relayer.rs`,
    // standing up three libp2p swarms over native transports so
    // subscribe propagation, mesh formation, and fan-out are real.
    #[test]
    #[ignore = "superseded by roam/tests/m6_via_relayer.rs — real-wire libp2p test"]
    fn m6_pickup_propagates_from_picker_to_observer() {
        use crate::teranos::{flower_at, surface_z, tile_at, TileKind, WORLD_CIRC_X};
        use crate::world::{World, INPUT_D, PIXELS_PER_TILE};
        use std::cell::RefCell;
        use std::rc::Rc;

        /// A's publish lands as a fresh `NetEvent::Message` on B's
        /// inbound queue. The PeerId in `from` is the sending side's
        /// identity, matching what gossipsub's signed-source semantics
        /// would deliver in production.
        struct MeshProvider {
            id: PeerId,
            out_to_peer: Rc<RefCell<Vec<NetEvent>>>,
            in_from_peer: Rc<RefCell<Vec<NetEvent>>>,
        }
        impl NetworkProvider for MeshProvider {
            fn identity(&self) -> PeerId {
                self.id.clone()
            }
            fn publish(&mut self, topic: &Topic, bytes: &[u8]) -> Result<(), NetError> {
                self.out_to_peer.borrow_mut().push(NetEvent::Message {
                    topic: topic.clone(),
                    from: Author(self.id.clone()),
                    bytes: bytes.to_vec(),
                    at_ms: 1_700_000_000_000,
                });
                Ok(())
            }
            fn subscribe(&mut self, _topic: &Topic) -> Result<(), NetError> {
                Ok(())
            }
            fn unsubscribe(&mut self, _topic: &Topic) -> Result<(), NetError> {
                Ok(())
            }
            fn poll_events(&mut self) -> Vec<NetEvent> {
                let mut guard = self.in_from_peer.borrow_mut();
                std::mem::take(&mut *guard)
            }
        }

        let mailbox_a_to_b: Rc<RefCell<Vec<NetEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let mailbox_b_to_a: Rc<RefCell<Vec<NetEvent>>> = Rc::new(RefCell::new(Vec::new()));

        let mut world_a = World::new();
        let mut world_b = World::new();
        world_a.net = Some(Net::new(Box::new(MeshProvider {
            id: PeerId("12D3KooWAlice".into()),
            out_to_peer: Rc::clone(&mailbox_a_to_b),
            in_from_peer: Rc::clone(&mailbox_b_to_a),
        })));
        world_b.net = Some(Net::new(Box::new(MeshProvider {
            id: PeerId("12D3KooWBob".into()),
            out_to_peer: Rc::clone(&mailbox_b_to_a),
            in_from_peer: Rc::clone(&mailbox_a_to_b),
        })));

        // Find a deterministic flower tile whose WEST neighbor is
        // walkable (not deep water / polar ocean) and not a cliff
        // (|Δz| ≤ 1). The realistic scenario is "walk onto a flower
        // from the adjacent tile" — this guarantees the walk works.
        let (fx, fy) = (|| {
            for ty in -5..=5 {
                for tx in 1..40 {
                    if flower_at(tx, ty).is_none() {
                        continue;
                    }
                    let sz_west = surface_z(tx - 1, ty);
                    let sz_flower = surface_z(tx, ty);
                    if sz_west < 0 || sz_flower < 0 {
                        continue;
                    }
                    if (sz_west - sz_flower).abs() > 1 {
                        continue;
                    }
                    // Confirm both surface tiles are walkable kinds —
                    // sanity check on top of the column_target_z proxies.
                    if matches!(tile_at(tx - 1, ty, sz_west.max(0)), TileKind::DeepWater) {
                        continue;
                    }
                    return (tx, ty);
                }
            }
            panic!("no walkable-west flower tile in scan window");
        })();
        let cx = fx.rem_euclid(WORLD_CIRC_X);

        // Place A one tile west of the flower, z matched to the
        // starting tile so try_set_position accepts the eastward step.
        let west_x_px = ((fx - 1) as f32 + 0.5) * PIXELS_PER_TILE as f32;
        let row_y_px = (fy as f32 + 0.5) * PIXELS_PER_TILE as f32;
        world_a.player.x = west_x_px;
        world_a.player.y = row_y_px;
        world_a.player.z = surface_z(fx - 1, fy).max(0);

        // === Action — A walks east onto the flower tile. ===
        // 200ms at SPEED=0.2 px/ms = 40 px east, more than one tile
        // (32 px). The step crosses the tile boundary, so try_pickup
        // fires with the player's CURRENT tile = (fx, fy).
        world_a.step(INPUT_D, 200.0);
        assert!(
            (world_a.player.x / PIXELS_PER_TILE as f32).floor() as i32 == fx,
            "precondition: A walked onto the flower tile",
        );

        // A2 — inventory grows.
        assert_eq!(world_a.player.inventory.len(), 1, "A2: A's inventory grows by 1");
        // A's canonical layer claim.
        assert!(world_a.canonical_picked.contains(&(cx, fy)), "A claims tile canonically");
        // A's personal record.
        assert!(world_a.player.picked.contains(&(cx, fy)), "A records personal pickup");
        // A4 — exactly one pickup message published.
        assert_eq!(
            mailbox_a_to_b.borrow().len(),
            1,
            "A4: A publishes exactly one pickup message",
        );
        // Wire shape check — the message body decodes as PickupWireIn.
        if let NetEvent::Message { topic, bytes, .. } = &mailbox_a_to_b.borrow()[0] {
            assert_eq!(topic.0, PICKUPS_TOPIC, "A publishes on the pickups topic");
            let decoded: serde_json::Value = serde_json::from_slice(bytes)
                .expect("A's payload must be valid JSON");
            assert_eq!(decoded["x"], cx);
            assert_eq!(decoded["y"], fy);
        } else {
            panic!("mailbox entry must be NetEvent::Message");
        }

        // === Observer side — Net.tick consumes inbound, drain applies
        // to canonical_picked exactly as the production FFI does in
        // `roam_net_tick`. ===
        let net_b = world_b.net.as_mut().expect("B has a Net");
        net_b.tick(1_700_000_000_000);
        let pickups = net_b.drain_pending_canonical_pickups();
        for (x, y) in pickups {
            world_b.canonical_picked.insert((x, y));
        }

        // B2 — canonical layer applied.
        assert!(
            world_b.canonical_picked.contains(&(cx, fy)),
            "B2: B's canonical layer accepts the gossipsub-propagated pickup",
        );
        // B3 — inventory unchanged.
        assert_eq!(
            world_b.player.inventory.len(),
            0,
            "B3: B's inventory does NOT change — B did not pick the flower",
        );
        // B4 — personal record clean.
        assert!(
            !world_b.player.picked.contains(&(cx, fy)),
            "B4: B's player.picked must not record a peer's pickup",
        );

        // B5 — the renderer's exclusion condition (mirrors viewport.rs).
        let cell_flower_excluded = if world_b.player.picked.contains(&(cx, fy))
            || world_b.canonical_picked.contains(&(cx, fy))
        {
            None
        } else {
            flower_at(fx, fy)
        };
        assert_eq!(
            cell_flower_excluded, None,
            "B5: B's renderer must exclude the flower — canonical_picked hides it",
        );

        // === Stronger end-state proof: B walks to the SAME tile and
        // tries to pick. canonical_picked must block try_pickup. B's
        // inventory and personal-picked stay clean.
        // This is the "A picks first, B can't pick after" invariant. ===
        world_b.player.x = west_x_px;
        world_b.player.y = row_y_px;
        world_b.player.z = surface_z(fx - 1, fy).max(0);

        world_b.step(INPUT_D, 200.0);
        assert!(
            (world_b.player.x / PIXELS_PER_TILE as f32).floor() as i32 == fx,
            "precondition: B walked onto the same flower tile",
        );

        assert_eq!(
            world_b.player.inventory.len(),
            0,
            "B-blocked: B walks onto the canonical-claimed tile and gets NOTHING",
        );
        assert!(
            !world_b.player.picked.contains(&(cx, fy)),
            "B-blocked: B's personal picked-set stays clean — try_pickup short-circuited",
        );
        assert_eq!(
            mailbox_b_to_a.borrow().len(),
            0,
            "B-blocked: B publishes ZERO messages — no double-claim broadcast",
        );
    }
}
