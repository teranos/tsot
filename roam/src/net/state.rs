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

use crate::net::{NetError, NetEvent, NetworkProvider, PeerId, Topic};

/// Topic where roam broadcasts player positions. Same string used by
/// every node — gossipsub treats this as a flat namespace.
pub const POSITIONS_TOPIC: &str = "roam-positions/v1";

/// One remote peer's last-known state. Rendered as a marker on the
/// world canvas in phase 2d. Stale entries (peers we haven't heard
/// from in `PEER_TIMEOUT_MS`) are pruned in `Net::tick` to avoid
/// rendering ghosts.
#[derive(Clone, Debug, PartialEq)]
pub struct RemotePeer {
    pub peer_id: PeerId,
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
    peers: BTreeMap<PeerId, RemotePeer>,
    /// Monotonic counter that bumps whenever the peer table changes
    /// — insert, remove, or position update. The render bridge reads
    /// this to feed the dirty-flag fingerprint so peers moving on
    /// screen actually triggers a repaint when the local player is
    /// still. `wrapping_add` so 32-bit u32 dirty-flag stays cheap to
    /// read across the FFI without BigInt-marshaling at the boundary.
    peer_state_seq: u32,
}

impl Net {
    pub fn new(provider: Box<dyn NetworkProvider>) -> Self {
        Self {
            provider,
            peers: BTreeMap::new(),
            peer_state_seq: 0,
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
                NetEvent::PeerDown { peer, .. } => {
                    if self.peers.remove(&peer).is_some() {
                        self.peer_state_seq = self.peer_state_seq.wrapping_add(1);
                    }
                }
                NetEvent::PeerUp { .. } | NetEvent::SubscriptionChange { .. } => {}
                NetEvent::Error(err) => {
                    // Routine network errors (publish-duplicate, no-peers,
                    // dial failures during normal operation) belong in the
                    // log, NOT the cursor popover. Same anti-pattern fix
                    // as `d21a533` on the JS-libp2p path: visibility yes,
                    // attention-grab no. The trace bus is the right surface
                    // — these events flow into the event-log panel the
                    // user already reads.
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
        let bytes = serde_json::to_vec(&msg).map_err(|e| NetError::ProviderInternal {
            reason: format!("position encode failed: {e}"),
        })?;
        self.provider.publish(&Topic(POSITIONS_TOPIC.to_string()), &bytes)
    }

    /// Subscribe to the canonical positions topic. Called once from
    /// the FFI's `roam_net_init`.
    pub fn subscribe_positions(&mut self) -> Result<(), NetError> {
        self.provider.subscribe(&Topic(POSITIONS_TOPIC.to_string()))
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
    fn net_tick_is_a_noop_in_phase_2a() {
        // Verifies the placeholder doesn't crash on a provider that
        // produces no events. Replace once 2b adds real logic.
        let mut net = Net::new(Box::new(StubProvider::new("12D3KooWself")));
        net.tick(1_700_000_000_000);
        assert_eq!(net.peers().count(), 0);
    }
}
