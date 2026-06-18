//! `RustLibp2pProvider` — direct rust-libp2p impl of `NetworkProvider`.
//!
//! - **3a** (done): stub returning `NotConnected` for every operation.
//! - **3b.1** (done): pinned deps, both feature configs link to native
//!   + wasm32-unknown-unknown.
//! - **3b.2** (this commit): real Swarm. WebSocket-WebSys + WebRTC-
//!   WebSys transport stack, gossipsub + identify + ping behaviour,
//!   `wasm_bindgen_futures::spawn_local` driver task, command channel
//!   + event queue routed through `NetworkProvider`.
//! - **3b.3** (next): JS bridge wiring + dial bootstrap relay + end-to-
//!   end parity test against a `JsLibp2pProvider` peer.
//!
//! Native (non-wasm32) build keeps the 3a stub shape so unit tests
//! continue to exercise the trait surface. The real Swarm only
//! compiles on wasm32 because `wasm_bindgen_futures::spawn_local` and
//! the websys transports are browser-only.

use crate::net::{NetError, NetEvent, NetworkProvider, PeerId, Topic};

// ---------- native (test) stub ----------------------------------------

#[cfg(not(target_arch = "wasm32"))]
pub struct RustLibp2pProvider {
    self_peer_id: PeerId,
}

#[cfg(not(target_arch = "wasm32"))]
impl RustLibp2pProvider {
    /// Native (test) constructor. The real `new()` exists only on wasm32.
    pub fn new_stub(self_peer_id: PeerId) -> Self {
        Self { self_peer_id }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl NetworkProvider for RustLibp2pProvider {
    fn identity(&self) -> PeerId {
        self.self_peer_id.clone()
    }
    fn publish(&mut self, _topic: &Topic, _bytes: &[u8]) -> Result<(), NetError> {
        Err(NetError::NotConnected {
            reason: "RustLibp2pProvider stub (non-wasm build)".into(),
        })
    }
    fn subscribe(&mut self, _topic: &Topic) -> Result<(), NetError> {
        Err(NetError::NotConnected {
            reason: "RustLibp2pProvider stub (non-wasm build)".into(),
        })
    }
    fn unsubscribe(&mut self, _topic: &Topic) -> Result<(), NetError> {
        Err(NetError::NotConnected {
            reason: "RustLibp2pProvider stub (non-wasm build)".into(),
        })
    }
    fn poll_events(&mut self) -> Vec<NetEvent> {
        Vec::new()
    }
}

// ---------- wasm32 (real) ---------------------------------------------

#[cfg(target_arch = "wasm32")]
mod real {
    use super::*;
    use futures::channel::mpsc;
    use futures::{select_biased, StreamExt};
    use libp2p::core::transport::Transport as _;
    use libp2p::core::upgrade;
    use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
    use libp2p::{gossipsub, identify, identity, ping, Swarm, SwarmBuilder};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    /// Composite behaviour. The `NetworkBehaviour` derive synthesises
    /// `RoamBehaviourEvent` (one variant per sub-behaviour) which is
    /// what the swarm event loop matches against.
    #[derive(NetworkBehaviour)]
    struct RoamBehaviour {
        gossipsub: gossipsub::Behaviour,
        identify: identify::Behaviour,
        ping: ping::Behaviour,
    }

    /// Commands from the trait surface into the swarm driver task.
    enum Cmd {
        Publish { topic: Topic, bytes: Vec<u8> },
        Subscribe(Topic),
        Unsubscribe(Topic),
    }

    pub struct RustLibp2pProvider {
        self_peer_id: PeerId,
        cmd_tx: mpsc::UnboundedSender<Cmd>,
        events: Rc<RefCell<Vec<NetEvent>>>,
    }

    impl RustLibp2pProvider {
        /// Construct a real provider. Generates a fresh ed25519 identity
        /// each session (persistent identity is a future concern).
        /// `bootstrap_addrs` are dialed by the driver task immediately
        /// after the Swarm starts; failures are surfaced as
        /// `NetEvent::Error` and don't block construction.
        /// Spawns the Swarm driver via `wasm_bindgen_futures::spawn_local`;
        /// the task runs until `cmd_tx` is dropped (provider drop).
        pub fn new(bootstrap_addrs: Vec<String>) -> Result<Self, NetError> {
            let keypair = identity::Keypair::generate_ed25519();
            let peer_id = libp2p::PeerId::from(keypair.public());
            let self_peer_id = PeerId(peer_id.to_string());

            // Gossipsub: use the default message-id function, which
            // combines source peer-id + sequence number. Hashing only
            // `msg.data` (what I had before) means identical position
            // payloads get identical IDs and are rejected as
            // `Duplicate` locally before they even hit the mesh — the
            // player standing still would never publish a second time.
            // js-libp2p uses the same default and that's why tab-js's
            // duplicates DO propagate.
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(10))
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
                "/roam/1.0.0".to_string(),
                keypair.public(),
            ));

            let ping_b = ping::Behaviour::new(ping::Config::default());

            let behaviour = RoamBehaviour {
                gossipsub: gossipsub_b,
                identify: identify_b,
                ping: ping_b,
            };

            // Transport stack: WebSocket(noise+yamux) ⊕ WebRTC(self-secured).
            // Both muxers are erased into `StreamMuxerBox` so the
            // combined transport has a single `(PeerId, StreamMuxerBox)`
            // output. `or_transport` yields `Either<L, R>` of the two
            // outputs; since both are now the same shape, the final
            // `.map` flattens it.
            let swarm: Swarm<RoamBehaviour> = SwarmBuilder::with_existing_identity(keypair)
                .with_wasm_bindgen()
                .with_other_transport(|key| {
                    let ws = libp2p::websocket_websys::Transport::default()
                        .upgrade(upgrade::Version::V1)
                        .authenticate(
                            libp2p::noise::Config::new(key).expect("noise config from keypair"),
                        )
                        .multiplex(libp2p::yamux::Config::default())
                        .map(|(p, m), _| {
                            (p, libp2p::core::muxing::StreamMuxerBox::new(m))
                        });
                    let webrtc = libp2p::webrtc_websys::Transport::new(
                        libp2p::webrtc_websys::Config::new(key),
                    )
                    .map(|(p, m), _| (p, libp2p::core::muxing::StreamMuxerBox::new(m)));
                    let combined = ws.or_transport(webrtc).map(|either, _| match either {
                        futures::future::Either::Left(t) => t,
                        futures::future::Either::Right(t) => t,
                    });
                    Ok(combined.boxed())
                })
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("swarm transport: {e}"),
                })?
                .with_behaviour(|_key| behaviour)
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("swarm behaviour: {e}"),
                })?
                // Default connection_timeout is 10s — covers the entire
                // upgrade stack (TCP+WS open + multistream-select + noise +
                // yamux). In wasm32 main-thread context the raw WS open
                // alone has been measured at 2-16s in headless Chromium,
                // leaving zero budget for the rest of the upgrade.
                // Bumping to 60s removes the per-stage time pressure so
                // we can isolate whether timeout was the cause vs there
                // being deeper protocol issues.
                // `with_connection_timeout` lives on `BuildPhase`, so it
                // must come AFTER `with_swarm_config` (which transitions
                // SwarmPhase → BuildPhase).
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
    }

    impl NetworkProvider for RustLibp2pProvider {
        fn identity(&self) -> PeerId {
            self.self_peer_id.clone()
        }

        fn publish(&mut self, topic: &Topic, bytes: &[u8]) -> Result<(), NetError> {
            self.cmd_tx
                .unbounded_send(Cmd::Publish {
                    topic: topic.clone(),
                    bytes: bytes.to_vec(),
                })
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("publish cmd send: {e}"),
                })
        }

        fn subscribe(&mut self, topic: &Topic) -> Result<(), NetError> {
            self.cmd_tx
                .unbounded_send(Cmd::Subscribe(topic.clone()))
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("subscribe cmd send: {e}"),
                })
        }

        fn unsubscribe(&mut self, topic: &Topic) -> Result<(), NetError> {
            self.cmd_tx
                .unbounded_send(Cmd::Unsubscribe(topic.clone()))
                .map_err(|e| NetError::ProviderInternal {
                    reason: format!("unsubscribe cmd send: {e}"),
                })
        }

        fn poll_events(&mut self) -> Vec<NetEvent> {
            std::mem::take(&mut *self.events.borrow_mut())
        }
    }

    async fn drive_swarm(
        mut swarm: Swarm<RoamBehaviour>,
        mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
        events: Rc<RefCell<Vec<NetEvent>>>,
        bootstrap_addrs: Vec<String>,
    ) {
        // Dial bootstrap addrs once at startup. Failures surface as
        // `NetEvent::Error` and the loop continues; the redial story
        // here is provider-internal (libp2p connection-keepalive +
        // explicit retry) and lands when bootstrap stability needs it.
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

    fn handle_swarm_event(event: SwarmEvent<RoamBehaviourEvent>, events: &Rc<RefCell<Vec<NetEvent>>>) {
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
                        .map(|e| format!("{e:?}"))
                        .unwrap_or_else(|| "closed".into()),
                });
            }
            // Dial failures — surfaced so we can see WHY the mesh
            // doesn't form. Without this, an `OutgoingConnectionError`
            // is silently swallowed by the catch-all and we're left
            // guessing why `peers=0`.
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                let peer_str = peer_id
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "<unknown>".into());
                events
                    .borrow_mut()
                    .push(NetEvent::Error(NetError::NotConnected {
                        reason: format!("outgoing dial to {peer_str}: {error}"),
                    }));
            }
            SwarmEvent::IncomingConnectionError { error, .. } => {
                events
                    .borrow_mut()
                    .push(NetEvent::Error(NetError::NotConnected {
                        reason: format!("incoming connection error: {error}"),
                    }));
            }
            SwarmEvent::Dialing { peer_id, .. } => {
                let peer_str = peer_id
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "<unknown>".into());
                events
                    .borrow_mut()
                    .push(NetEvent::Error(NetError::NotConnected {
                        reason: format!("dialing {peer_str}…"),
                    }));
            }
            SwarmEvent::Behaviour(RoamBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                propagation_source,
                message,
                ..
            })) => {
                events.borrow_mut().push(NetEvent::Message {
                    topic: Topic(message.topic.to_string()),
                    from: PeerId(propagation_source.to_string()),
                    bytes: message.data,
                    at_ms: now_ms(),
                });
            }
            SwarmEvent::Behaviour(RoamBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed {
                peer_id,
                topic,
            })) => {
                events.borrow_mut().push(NetEvent::SubscriptionChange {
                    topic: Topic(topic.to_string()),
                    peer: PeerId(peer_id.to_string()),
                    joined: true,
                });
            }
            SwarmEvent::Behaviour(RoamBehaviourEvent::Gossipsub(gossipsub::Event::Unsubscribed {
                peer_id,
                topic,
            })) => {
                events.borrow_mut().push(NetEvent::SubscriptionChange {
                    topic: Topic(topic.to_string()),
                    peer: PeerId(peer_id.to_string()),
                    joined: false,
                });
            }
            _ => {}
        }
    }

    fn now_ms() -> u64 {
        // Browser-only — std::time::SystemTime panics in wasm without
        // the wasi-clock backend. js_sys::Date::now() returns f64 ms.
        js_sys::Date::now() as u64
    }
}

#[cfg(target_arch = "wasm32")]
pub use real::RustLibp2pProvider;

// ---------- tests (native stub only) ----------------------------------

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_supplied_identity() {
        let p = RustLibp2pProvider::new_stub(PeerId("12D3KooWrust".into()));
        assert_eq!(p.identity().0, "12D3KooWrust");
    }

    #[test]
    fn stub_publish_reports_not_connected() {
        let mut p = RustLibp2pProvider::new_stub(PeerId("12D3KooWrust".into()));
        let topic = Topic("roam-positions/v1".into());
        match p.publish(&topic, &[1, 2, 3]) {
            Err(NetError::NotConnected { .. }) => {}
            other => panic!("expected NotConnected, got {other:?}"),
        }
    }

    #[test]
    fn stub_subscribe_reports_not_connected() {
        let mut p = RustLibp2pProvider::new_stub(PeerId("12D3KooWrust".into()));
        let topic = Topic("roam-positions/v1".into());
        assert!(matches!(
            p.subscribe(&topic),
            Err(NetError::NotConnected { .. })
        ));
    }

    #[test]
    fn stub_unsubscribe_reports_not_connected() {
        let mut p = RustLibp2pProvider::new_stub(PeerId("12D3KooWrust".into()));
        let topic = Topic("roam-positions/v1".into());
        assert!(matches!(
            p.unsubscribe(&topic),
            Err(NetError::NotConnected { .. })
        ));
    }

    #[test]
    fn stub_poll_events_is_empty() {
        let mut p = RustLibp2pProvider::new_stub(PeerId("12D3KooWrust".into()));
        assert!(p.poll_events().is_empty());
    }

    #[test]
    fn provider_is_object_safe_through_box() {
        let p: Box<dyn NetworkProvider> =
            Box::new(RustLibp2pProvider::new_stub(PeerId("12D3KooWrust".into())));
        assert_eq!(p.identity().0, "12D3KooWrust");
    }
}
