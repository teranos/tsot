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

use serde::Serialize;

use crate::net::{NetError, NetworkProvider, PeerId, Topic};

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
}

impl Net {
    pub fn new(provider: Box<dyn NetworkProvider>) -> Self {
        Self {
            provider,
            peers: BTreeMap::new(),
        }
    }

    /// Read-only iteration over the peer table. Used by the renderer
    /// in phase 2d to build the marker instance buffer; today (2a)
    /// no caller exists.
    pub fn peers(&self) -> impl Iterator<Item = &RemotePeer> {
        self.peers.values()
    }

    /// Called once per frame from `World::step` in phase 2b. Drains
    /// provider events, updates the peer table, prunes timed-out
    /// peers, schedules broadcasts.
    ///
    /// Phase 2a: placeholder. The provider-events drain and the prune
    /// loop are the next slice; today this exists so 2b is a one-line
    /// embedding in `World::step`.
    pub fn tick(&mut self, now_ms: u64) {
        // Phase 2b: handle each event from `self.provider.poll_events()`
        //   - NetEvent::PeerUp     → insert/update peer entry
        //   - NetEvent::PeerDown   → remove peer
        //   - NetEvent::Message    → decode WireMsg::Position, update peer
        //   - NetEvent::Error      → forward to `error::push`
        // Phase 2b: prune `self.peers` entries with `last_seen_ms`
        //   older than `now_ms - PEER_TIMEOUT_MS`.
        let _ = now_ms;
        let _ = self.provider.poll_events();
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
