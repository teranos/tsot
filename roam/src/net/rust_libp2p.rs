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
    use libp2p_connection_limits as connection_limits;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    // IDENTITY MENU (roam/IDENTITY.md):
    //   A2 — read W3C did:key spec; confirm Ed25519 → did:key:z6Mk…
    //   A3 — PeerId and did:key derive from the same 32-byte pubkey, different encodings.
    //   A8 — trace from this function to PeerId emission; draw the data flow.
    //   S6 — wasm-bindgen-test asserting PeerId ↔ did:key round-trip on this keypair.
    //   S8 — failing native test for this function (extract out of the wasm-only gate).
    //   M1 — adopt did:key as the project's primary identifier; PeerId becomes detail.
    //   C3 — move identity code into roam/src/identity/ as its own module.
    //   C6 — audit every Keypair::generate_ed25519() call site, including the fall-through below.
    /// Decode the libp2p-canonical protobuf-encoded keypair the JS
    /// bridge loaded from IndexedDB. None → generate fresh (the
    /// bridge will persist the bytes after this call returns so the
    /// next session loads them back). Refusing to fall through to
    /// "generate fresh" on a decode failure is deliberate: a corrupt
    /// stored identity should surface as an error, not silently
    /// rotate the PeerId behind the user's back.
    fn load_or_generate_keypair(bytes: Option<&[u8]>) -> Result<identity::Keypair, NetError> {
        match bytes {
            Some(b) => identity::Keypair::from_protobuf_encoding(b).map_err(|e| {
                NetError::ProviderInternal {
                    reason: format!("identity decode: {e}"),
                }
            }),
            None => Ok(identity::Keypair::generate_ed25519()),
        }
    }

    /// Compose-up a fresh Ed25519 keypair and return its libp2p-
    /// canonical protobuf encoding. The JS bridge calls this once on
    /// first visit (when IndexedDB has no `roam/identity/v1` entry),
    /// stores the returned bytes, and passes them to every subsequent
    /// `roam_net_worker_provider_init` call so PeerId is stable
    /// across sessions.
    pub fn generate_identity_protobuf() -> Result<Vec<u8>, NetError> {
        identity::Keypair::generate_ed25519()
            .to_protobuf_encoding()
            .map_err(|e| NetError::ProviderInternal {
                reason: format!("identity encode: {e}"),
            })
    }

    /// Composite behaviour. The `NetworkBehaviour` derive synthesises
    /// `RoamBehaviourEvent` (one variant per sub-behaviour) which is
    /// what the swarm event loop matches against.
    #[derive(NetworkBehaviour)]
    struct RoamBehaviour {
        gossipsub: gossipsub::Behaviour,
        identify: identify::Behaviour,
        ping: ping::Behaviour,
        // Enforces "only the relay" architecturally. roam doesn't
        // need a wide mesh — the relay subscribes to the same topic
        // and re-broadcasts, so a single connected peer is sufficient.
        // See `CANONICAL.md` for the design (canonical world via
        // relay, non-canonical sandbox local only).
        connection_limits: connection_limits::Behaviour,
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
        /// Construct a real provider. `identity_bytes` is the
        /// libp2p-canonical protobuf-encoded keypair the JS bridge
        /// loaded from IndexedDB; pass `None` to generate fresh (the
        /// bridge does this on first visit then persists the bytes).
        /// `bootstrap_addrs` are dialed by the driver task immediately
        /// after the Swarm starts; failures are surfaced as
        /// `NetEvent::Error` and don't block construction.
        /// Spawns the Swarm driver via `wasm_bindgen_futures::spawn_local`;
        /// the task runs until `cmd_tx` is dropped (provider drop).
        pub fn new(
            bootstrap_addrs: Vec<String>,
            identity_bytes: Option<&[u8]>,
        ) -> Result<Self, NetError> {
            let keypair = load_or_generate_keypair(identity_bytes)?;
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
                // 1s matches js-libp2p's default and the rate the relay
                // sets (matches the relayer's `GOSSIPSUB_HEARTBEAT` constant).
                // 10s left mesh maintenance lagging long enough that the
                // connection died with BrokenPipe before GRAFT could
                // settle. Empirically observed via the inspect.ts probe
                // on 2026-06-18.
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
                "/roam/1.0.0".to_string(),
                keypair.public(),
            ));

            // Ping every 5s instead of the 15s default — keeps the
            // WebSocket warm against any idle-timeout on the relay's
            // network path (CloudFront / Lightsail / kernel).
            let ping_b = ping::Behaviour::new(
                ping::Config::new().with_interval(Duration::from_secs(5)),
            );

            // "Only the relay" cap. One established outgoing connection
            // (the bootstrap relay), zero incoming (browsers can't
            // accept inbound), a small pending-outgoing window so the
            // bootstrap retry has room. Without this cap, identify-
            // discovered addresses become auto-dial targets that all
            // fail (browsers can't reach each other without WebRTC,
            // which we dropped), producing the `net::provider_error`
            // stream that hid every other signal in the event log.
            let conn_limits = connection_limits::ConnectionLimits::default()
                .with_max_established_outgoing(Some(1))
                .with_max_established_incoming(Some(0))
                .with_max_established_per_peer(Some(1))
                .with_max_pending_outgoing(Some(2));
            let connection_limits_b = connection_limits::Behaviour::new(conn_limits);

            let behaviour = RoamBehaviour {
                gossipsub: gossipsub_b,
                identify: identify_b,
                ping: ping_b,
                connection_limits: connection_limits_b,
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
                    // WebSocket only. Previous code also added
                    // `libp2p::webrtc_websys::Transport`, which calls
                    // `web_sys::RtcPeerConnection::new_with_configuration`
                    // (see ~/.cargo/.../libp2p-webrtc-websys-*/src/
                    // connection.rs `RtcPeerConnection::new`) at upgrade
                    // time — this maps to a JS-side `new RTCPeerConnection(…)`.
                    //
                    // The capability probe at the top of
                    // `assets/src/net-worker.js` empirically reported
                    // `RTCPeerConnection=absent` in dedicated workers
                    // in BOTH Firefox and Chrome on 2026-06-18
                    // (versions current at that date). It is therefore
                    // worker-context-general at least for those two
                    // engines today, not Firefox-specific as an earlier
                    // version of this comment claimed.
                    //
                    // Authoritative cross-vendor citation TODO — the
                    // most recent attempt to pull MDN compat data or a
                    // tracking bug timed out / didn't surface a
                    // single-line spec quote. Until then, the canonical
                    // evidence is the probe in `net-worker.js` — re-run
                    // it before reintroducing WebRTC. The Chromium
                    // tracker for the work is referenced as issue 40262971
                    // ("Using WebRTC inside Service Worker"); it is NOT
                    // a worker support announcement, treat it as a
                    // breadcrumb not a guarantee.
                    //
                    // WebSocket via the relay is the substrate the JS
                    // path already uses, so this is functional parity
                    // for the shipping behaviour, not a downgrade.
                    let ws = libp2p::websocket_websys::Transport::default()
                        .upgrade(upgrade::Version::V1)
                        .authenticate(
                            libp2p::noise::Config::new(key).expect("noise config from keypair"),
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

            // Heartbeat probe — separate task that sleeps 100ms then
            // wakes. Each wake records gap_ms since the last wake; every
            // ~1s of intended elapsed time, emits a trace with the max
            // gap seen. If the main thread is starving futures, max_gap
            // grows well past 100ms. If scheduling is healthy, max_gap
            // hovers near 100ms (the requested sleep interval).
            wasm_bindgen_futures::spawn_local(heartbeat_task());

            Ok(Self {
                self_peer_id,
                cmd_tx,
                events,
            })
        }
    }

    async fn heartbeat_task() {
        let start_ms = now_ms();
        let mut last_wake_ms = start_ms;
        let mut wake_count = 0u64;
        let mut max_gap_ms: u64 = 0;
        loop {
            futures_timer::Delay::new(Duration::from_millis(100)).await;
            let now = now_ms();
            let gap = now.saturating_sub(last_wake_ms);
            if gap > max_gap_ms {
                max_gap_ms = gap;
            }
            last_wake_ms = now;
            wake_count += 1;
            // Every 10 wakes ≈ 1s wall-clock (if not starved).
            if wake_count.is_multiple_of(10) {
                crate::trace::emit(crate::trace::TraceEvent::Note {
                    tag: "net::heartbeat",
                    msg: format!(
                        "wakes={} elapsed_ms={} max_gap_ms_window={}",
                        wake_count,
                        now.saturating_sub(start_ms),
                        max_gap_ms,
                    ),
                });
                max_gap_ms = 0;
            }
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
        crate::trace::emit(crate::trace::TraceEvent::Note {
            tag: "net::drive_swarm_start",
            msg: format!("bootstrap_addrs.len()={}", bootstrap_addrs.len()),
        });
        for addr_str in &bootstrap_addrs {
            crate::trace::emit(crate::trace::TraceEvent::Note {
                tag: "net::dial_attempt",
                msg: addr_str.clone(),
            });
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
    /// surfacing
    /// `IO(Custom { kind: Other, error: Custom { kind: Other, error: Error(Right(Decode(Io(Kind(BrokenPipe))))) } })`
    /// we surface `io::BrokenPipe ← yamux decode error ← <wrapper>`,
    /// which is what the user needs to diagnose without scrolling.
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
                        .as_ref()
                        .map(|e| decode_error_chain(e))
                        .unwrap_or_else(|| "graceful close (no cause reported)".into()),
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
            // `Dialing` is a STATUS event ("I'm initiating a dial"),
            // not a failure — it fires continuously during libp2p's
            // peer-discovery / mesh-repair / keepalive cycles. Pushing
            // it as `NetEvent::Error` produced a ~20Hz storm of
            // `net::provider_error` traces that drowned every other
            // signal in the event log. Actual dial failures are still
            // surfaced via `OutgoingConnectionError` above; successful
            // dials produce `ConnectionEstablished` → `PeerUp`. Dialing-
            // in-progress is redundant with both and not worth
            // surfacing.
            SwarmEvent::Dialing { .. } => {}
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
            // Ping behaviour — emits per outgoing ping result. The
            // single most diagnostic event for "did the connection
            // actually die or did we kill it." Both success (with
            // RTT) and failure are surfaced; rendering / dedup is
            // the bridge's responsibility.
            SwarmEvent::Behaviour(RoamBehaviourEvent::Ping(ping::Event { peer, result, .. })) => {
                match result {
                    Ok(rtt) => {
                        crate::trace::emit(crate::trace::TraceEvent::Note {
                            tag: "net::ping_ok",
                            msg: format!("peer={} rtt_ms={}", peer, rtt.as_millis()),
                        });
                    }
                    Err(failure) => {
                        crate::trace::emit(crate::trace::TraceEvent::Note {
                            tag: "net::ping_failed",
                            msg: format!("peer={} failure={}", peer, failure),
                        });
                    }
                }
            }
            // Identify protocol — peer info exchange. `Received` carries
            // the remote's advertised protocol version and listen addrs;
            // `Sent` confirms we pushed ours. `Error` is a real problem
            // (e.g., protocol version mismatch).
            SwarmEvent::Behaviour(RoamBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
                crate::trace::emit(crate::trace::TraceEvent::Note {
                    tag: "net::identify_received",
                    msg: format!(
                        "peer={peer_id} protocol={} agent={} listen_addrs={}",
                        info.protocol_version,
                        info.agent_version,
                        info.listen_addrs.len()
                    ),
                });
            }
            SwarmEvent::Behaviour(RoamBehaviourEvent::Identify(identify::Event::Sent {
                peer_id,
                ..
            })) => {
                crate::trace::emit(crate::trace::TraceEvent::Note {
                    tag: "net::identify_sent",
                    msg: format!("peer={peer_id}"),
                });
            }
            SwarmEvent::Behaviour(RoamBehaviourEvent::Identify(identify::Event::Error {
                peer_id,
                error,
                ..
            })) => {
                let chain = decode_error_chain(&error);
                crate::trace::emit(crate::trace::TraceEvent::Note {
                    tag: "net::identify_error",
                    msg: format!("peer={peer_id} {chain}"),
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
pub use real::{generate_identity_protobuf, RustLibp2pProvider};

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
