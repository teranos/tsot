//! Network surface for rave's libp2p slice.
//!
//! Pure-data types: PeerId, Topic, NetError, NetEvent, RavePosition.
//! Wire format is JSON via serde for the position broadcast at
//! `rave-positions/v1`. Errors never collapse — every variant of
//! NetError pins one cause.

use serde::{Deserialize, Serialize};

/// libp2p PeerId in its base58btc form (`12D3KooW…` for Ed25519 keys).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub String);

/// gossipsub topic name. Single string, no hashing — IdentTopic on the
/// libp2p side.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Topic(pub String);

/// One cause per variant. No collapse — each carries the context the
/// drawer needs to surface what the user should do next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetError {
    PublishFailed { topic: Topic, reason: String },
    SubscribeFailed { topic: Topic, reason: String },
    NotConnected { reason: String },
    InvalidTopic { topic: Topic, reason: String },
    ProviderInternal { reason: String },
}

/// Asynchronous events the Swarm task accumulates and the Bevy drain
/// system reads each frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetEvent {
    PeerUp {
        peer: PeerId,
        addrs: Vec<String>,
    },
    PeerDown {
        peer: PeerId,
        reason: String,
    },
    Message {
        topic: Topic,
        from: PeerId,
        bytes: Vec<u8>,
        at_ms: u64,
    },
    SubscriptionChange {
        topic: Topic,
        peer: PeerId,
        joined: bool,
    },
    Error(NetError),
}

/// Wire payload for `rave-positions/v1`. Player XYZ in world units,
/// peer's libp2p PeerId, wall-clock millis at publish.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RavePosition {
    pub peer: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub at_ms: u64,
}

/// Wire payload for `rave-chat/v1`. One line of chat from a peer.
/// `peer` is the libp2p PeerId of the sender (matches the signed
/// gossipsub source — the field is informational; trust the source).
/// `body` is UTF-8 plain text capped at 512 bytes by the publish
/// path. `at_ms` is wall-clock millis at publish.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaveChatMsg {
    pub peer: String,
    pub body: String,
    pub at_ms: u64,
}

// Real Swarm wiring — wasm32-only because the WebSocket-WebSys
// transport, gossipsub, and wasm_bindgen_futures::spawn_local don't
// compile for native. Adapted from roam/src/net/rust_libp2p.rs's
// production-verified setup (M5/M6).
#[cfg(target_arch = "wasm32")]
pub use real::Net;

#[cfg(target_arch = "wasm32")]
mod real {
    use super::{NetError, NetEvent, PeerId, Topic};
    use crate::identity;
    use futures::channel::mpsc;
    use futures::{select_biased, StreamExt};
    use libp2p::core::transport::Transport as _;
    use libp2p::core::upgrade;
    use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
    use libp2p::{gossipsub, identify, ping, Swarm, SwarmBuilder};
    use libp2p_connection_limits as connection_limits;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    /// Composite behaviour. The `NetworkBehaviour` derive synthesises
    /// `RaveBehaviourEvent` (one variant per sub-behaviour) which is
    /// what the swarm event loop matches against.
    #[derive(NetworkBehaviour)]
    struct RaveBehaviour {
        gossipsub: gossipsub::Behaviour,
        identify: identify::Behaviour,
        ping: ping::Behaviour,
        /// "Only the relay" cap. Browsers can't accept inbound; auto-
        /// dial of identify-discovered peers fails (no WebRTC). Without
        /// this cap, every failed dial surfaces as a NetEvent::Error and
        /// drowns the drawer.
        connection_limits: connection_limits::Behaviour,
    }

    enum Cmd {
        Publish { topic: Topic, bytes: Vec<u8> },
        Subscribe(Topic),
        Unsubscribe(Topic),
    }

    /// Application-layer libp2p facade. Owns the cmd sender + event
    /// queue; the actual Swarm runs on a spawn_local task in the
    /// browser's microtask loop. `!Send + !Sync` — Bevy holds it as a
    /// NonSend resource per wasm32's single-threaded reality.
    pub struct Net {
        self_peer_id: PeerId,
        cmd_tx: mpsc::UnboundedSender<Cmd>,
        events: Rc<RefCell<Vec<NetEvent>>>,
    }

    impl Net {
        /// `bootstrap_addrs` are dialed once after the Swarm starts.
        /// `identity_bytes` is the protobuf-encoded keypair the JS bridge
        /// restored from IndexedDB; `None` mints fresh (call site is
        /// responsible for persisting the fresh bytes after construction
        /// so the next session restores them).
        pub fn new(
            bootstrap_addrs: Vec<String>,
            identity_bytes: Option<&[u8]>,
        ) -> Result<Self, NetError> {
            let keypair = identity::load_or_generate_keypair(identity_bytes)?;
            let peer_id = libp2p::PeerId::from(keypair.public());
            let self_peer_id = PeerId(peer_id.to_string());

            // Gossipsub: 1s heartbeat matches the relayer's
            // GOSSIPSUB_HEARTBEAT. Strict validation rejects unsigned
            // messages (the relayer signs; peers do too).
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(1))
                .validation_mode(gossipsub::ValidationMode::Strict)
                .build()
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("gossipsub config: {e}"),
                })?;
            let gossipsub_b = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(keypair.clone()),
                gossipsub_config,
            )
            .map_err(|e| NetError::ProviderInternal {
                reason: format!("gossipsub behaviour: {e}"),
            })?;

            let identify_b = identify::Behaviour::new(identify::Config::new(
                "/rave/1.0.0".to_string(),
                keypair.public(),
            ));

            // 5s ping keeps the WebSocket warm against idle timeouts on
            // the relay's network path (CloudFront / Lightsail / kernel).
            let ping_b = ping::Behaviour::new(
                ping::Config::new().with_interval(Duration::from_secs(5)),
            );

            let conn_limits = connection_limits::ConnectionLimits::default()
                .with_max_established_outgoing(Some(1))
                .with_max_established_incoming(Some(0))
                .with_max_established_per_peer(Some(1))
                .with_max_pending_outgoing(Some(2));
            let connection_limits_b = connection_limits::Behaviour::new(conn_limits);

            let behaviour = RaveBehaviour {
                gossipsub: gossipsub_b,
                identify: identify_b,
                ping: ping_b,
                connection_limits: connection_limits_b,
            };

            // WebSocket-only transport. WebRTC-WebSys was tested in
            // roam and empirically rejected in dedicated workers across
            // Chrome+Firefox; functional parity with the JS path uses
            // WebSocket via the relay.
            let swarm: Swarm<RaveBehaviour> = SwarmBuilder::with_existing_identity(keypair)
                .with_wasm_bindgen()
                .with_other_transport(|key| {
                    let ws = libp2p::websocket_websys::Transport::default()
                        .upgrade(upgrade::Version::V1)
                        .authenticate(
                            libp2p::noise::Config::new(key)
                                .expect("noise config from keypair"),
                        )
                        .multiplex(libp2p::yamux::Config::default())
                        .map(|(p, m), _| {
                            (p, libp2p::core::muxing::StreamMuxerBox::new(m))
                        });
                    Ok(ws.boxed())
                })
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("swarm transport: {e}"),
                })?
                .with_behaviour(|_key| behaviour)
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("swarm behaviour: {e}"),
                })?
                // 60s idle + 60s connection_timeout. WebSocket open in
                // wasm32 main thread has been measured at 2-16s in
                // headless Chromium; default 10s left no budget for the
                // upgrade stack (noise + yamux).
                .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
                .with_connection_timeout(Duration::from_secs(60))
                .build();

            let (cmd_tx, cmd_rx) = mpsc::unbounded::<Cmd>();
            let events: Rc<RefCell<Vec<NetEvent>>> = Rc::new(RefCell::new(Vec::new()));

            wasm_bindgen_futures::spawn_local(drive_swarm(
                swarm,
                cmd_rx,
                events.clone(),
                bootstrap_addrs,
            ));

            Ok(Self {
                self_peer_id,
                cmd_tx,
                events,
            })
        }

        pub fn identity(&self) -> &PeerId {
            &self.self_peer_id
        }

        pub fn publish(&self, topic: &Topic, bytes: &[u8]) -> Result<(), NetError> {
            self.cmd_tx
                .unbounded_send(Cmd::Publish {
                    topic: topic.clone(),
                    bytes: bytes.to_vec(),
                })
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("publish cmd send: {e}"),
                })
        }

        pub fn subscribe(&self, topic: &Topic) -> Result<(), NetError> {
            self.cmd_tx
                .unbounded_send(Cmd::Subscribe(topic.clone()))
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("subscribe cmd send: {e}"),
                })
        }

        pub fn unsubscribe(&self, topic: &Topic) -> Result<(), NetError> {
            self.cmd_tx
                .unbounded_send(Cmd::Unsubscribe(topic.clone()))
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("unsubscribe cmd send: {e}"),
                })
        }

        pub fn poll_events(&self) -> Vec<NetEvent> {
            std::mem::take(&mut *self.events.borrow_mut())
        }
    }

    async fn drive_swarm(
        mut swarm: Swarm<RaveBehaviour>,
        mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
        events: Rc<RefCell<Vec<NetEvent>>>,
        bootstrap_addrs: Vec<String>,
    ) {
        for addr_str in &bootstrap_addrs {
            match addr_str.parse::<libp2p::Multiaddr>() {
                Ok(addr) => {
                    if let Err(e) = swarm.dial(addr) {
                        events
                            .borrow_mut()
                            .push(NetEvent::Error(NetError::NotConnected {
                                reason: format!("dial {addr_str}: {e}"),
                            }));
                    }
                }
                Err(e) => {
                    events
                        .borrow_mut()
                        .push(NetEvent::Error(NetError::ProviderInternal {
                            reason: format!("parse multiaddr {addr_str}: {e}"),
                        }));
                }
            }
        }
        loop {
            select_biased! {
                cmd = cmd_rx.next() => {
                    match cmd {
                        Some(Cmd::Publish { topic, bytes }) => {
                            let t = gossipsub::IdentTopic::new(&topic.0);
                            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(t, bytes) {
                                events.borrow_mut().push(NetEvent::Error(NetError::PublishFailed {
                                    topic,
                                    reason: format!("{e}"),
                                }));
                            }
                        }
                        Some(Cmd::Subscribe(topic)) => {
                            let t = gossipsub::IdentTopic::new(&topic.0);
                            if let Err(e) = swarm.behaviour_mut().gossipsub.subscribe(&t) {
                                events.borrow_mut().push(NetEvent::Error(NetError::SubscribeFailed {
                                    topic,
                                    reason: format!("{e}"),
                                }));
                            }
                        }
                        Some(Cmd::Unsubscribe(topic)) => {
                            let t = gossipsub::IdentTopic::new(&topic.0);
                            swarm.behaviour_mut().gossipsub.unsubscribe(&t);
                        }
                        None => break,
                    }
                }
                event = swarm.select_next_some() => {
                    handle_swarm_event(event, &events);
                }
            }
        }
    }

    /// Walk an error chain via `std::error::Error::source()`, extracting
    /// `io::ErrorKind` at each level where the type downcasts to
    /// `io::Error`. Stops `format!("{e:?}")`-style collapse — instead of
    /// surfacing wrapped Custom errors we surface the actual kinds.
    /// Bounded to 8 levels so a pathological cycle can't spin.
    fn decode_error_chain(err: &(dyn std::error::Error + 'static)) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
        let mut depth = 0;
        while let Some(e) = current {
            if depth >= 8 {
                parts.push("…(chain truncated)".into());
                break;
            }
            if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                parts.push(format!("io::{:?}", io_err.kind()));
            } else {
                let s = format!("{e}");
                if !s.is_empty() {
                    parts.push(s);
                }
            }
            current = e.source();
            depth += 1;
        }
        if parts.is_empty() {
            "(no detail)".into()
        } else {
            parts.join(" ← ")
        }
    }

    fn now_ms() -> u64 {
        js_sys::Date::now() as u64
    }

    fn handle_swarm_event(
        event: SwarmEvent<RaveBehaviourEvent>,
        events: &Rc<RefCell<Vec<NetEvent>>>,
    ) {
        match event {
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                events.borrow_mut().push(NetEvent::PeerUp {
                    peer: PeerId(peer_id.to_string()),
                    addrs: Vec::new(),
                });
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                events.borrow_mut().push(NetEvent::PeerDown {
                    peer: PeerId(peer_id.to_string()),
                    reason: cause
                        .as_ref()
                        .map(|e| decode_error_chain(e))
                        .unwrap_or_else(|| "graceful close (no cause reported)".into()),
                });
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                let peer_str = peer_id
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "<unknown>".into());
                let chain = decode_error_chain(&error);
                events
                    .borrow_mut()
                    .push(NetEvent::Error(NetError::NotConnected {
                        reason: format!("outgoing dial to {peer_str}: {chain}"),
                    }));
            }
            SwarmEvent::IncomingConnectionError { error, .. } => {
                let chain = decode_error_chain(&error);
                events
                    .borrow_mut()
                    .push(NetEvent::Error(NetError::NotConnected {
                        reason: format!("incoming connection error: {chain}"),
                    }));
            }
            SwarmEvent::Behaviour(RaveBehaviourEvent::Gossipsub(
                gossipsub::Event::Message {
                    propagation_source,
                    message,
                    ..
                },
            )) => {
                // Use the message's signed source if present (the
                // identity that signed the payload), falling back to
                // propagation_source. In a star topology the propagation
                // is always the relay, so signed source is what
                // distinguishes peers.
                let from = message
                    .source
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| propagation_source.to_string());
                events.borrow_mut().push(NetEvent::Message {
                    topic: Topic(message.topic.to_string()),
                    from: PeerId(from),
                    bytes: message.data,
                    at_ms: now_ms(),
                });
            }
            SwarmEvent::Behaviour(RaveBehaviourEvent::Gossipsub(
                gossipsub::Event::Subscribed { peer_id, topic },
            )) => {
                events.borrow_mut().push(NetEvent::SubscriptionChange {
                    topic: Topic(topic.to_string()),
                    peer: PeerId(peer_id.to_string()),
                    joined: true,
                });
            }
            SwarmEvent::Behaviour(RaveBehaviourEvent::Gossipsub(
                gossipsub::Event::Unsubscribed { peer_id, topic },
            )) => {
                events.borrow_mut().push(NetEvent::SubscriptionChange {
                    topic: Topic(topic.to_string()),
                    peer: PeerId(peer_id.to_string()),
                    joined: false,
                });
            }
            // Ping / Identify / ConnectionLimits behaviour events are
            // expected and uninteresting at the application layer. The
            // catch-all keeps the loop quiet.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(value: T)
    where
        T: serde::Serialize + for<'de> serde::Deserialize<'de> + PartialEq + std::fmt::Debug,
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
        round_trip(Topic("rave-positions/v1".to_string()));
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
    fn rave_position_round_trips() {
        round_trip(RavePosition {
            peer: "12D3KooWPeerSelf".into(),
            x: 1.5,
            y: 0.0,
            z: -3.2,
            at_ms: 1_700_000_000_000,
        });
    }

    #[test]
    fn rave_chat_msg_round_trips() {
        round_trip(RaveChatMsg {
            peer: "12D3KooWPeerSelf".into(),
            body: "hello from the dancefloor 🪩".into(),
            at_ms: 1_700_000_000_000,
        });
    }

    #[test]
    fn rave_chat_msg_empty_body_round_trips() {
        round_trip(RaveChatMsg {
            peer: "12D3KooWPeerSelf".into(),
            body: String::new(),
            at_ms: 0,
        });
    }
}
