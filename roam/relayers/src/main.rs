//! relayers — roam's Rust libp2p relayer.
//!
//! Replaces `roam/relay/relay.ts` + `roam/relay/bun-ws-transport.ts`
//! byte-for-byte on the wire so browsers see no change at cut-over.
//! See `roam/relay/relayers.md` for the migration plan and
//! acceptance criteria.
//!
//! What this binary does, in order:
//!
//! 1. Loads Ed25519 identity from AWS Secrets Manager (env
//!    `ROAM_RELAY_IDENTITY_SECRET`) so PeerId is stable across
//!    restarts. If the secret doesn't decode to a libp2p
//!    keypair, the binary refuses to start (no silent
//!    fall-through to a freshly-generated identity that would
//!    change the bootstrap address browsers know).
//! 2. Builds a `Swarm` with gossipsub, identify, ping, relay
//!    (circuit-relay-v2 server), over WebSocket+noise+yamux.
//! 3. Listens on the configured port (env
//!    `ROAM_RELAY_LISTEN_PORT`, default 9001) on
//!    `ROAM_RELAY_LISTEN_HOST` (default 0.0.0.0). Plain
//!    WebSocket; TLS terminates at CloudFront on the deployed
//!    box.
//! 4. Subscribes to the canonical topic (`roam-positions/v1`
//!    today; v0.4+ will add more) so it sees every message and
//!    gossipsub re-broadcasts to all subscribed peers.
//! 5. Publishes CloudWatch metrics every 60s on the same
//!    namespace + dimension the TS relay used.
//! 6. Logs connection lifecycle events at info-level by default;
//!    close codes always surfaced (no DEBUG drop-in dance).
//!
//! What this binary does NOT do (v1 = parity, not features):
//!
//! - No rate limiting per IP / per peer-id (a slice on top of v1)
//! - No UCAN / DID / capability verification (v0.4+)
//! - No multi-region / federation (later)

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use libp2p::{
    core::transport::Transport, futures::StreamExt, gossipsub, identify, identity, noise, ping,
    relay, swarm::SwarmEvent, websocket, yamux, Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use tracing::{info, warn};

mod identity_secret;
mod metrics;

/// Canonical positions topic. Must match the value the bridge
/// publishes on — see `roam/assets/src/js-bridge.js` `TOPIC`
/// constant and `roam/src/net/state.rs::POSITIONS_TOPIC`.
const POSITIONS_TOPIC: &str = "roam-positions/v1";

/// Canonical pickups topic. Must match `roam/src/net/state.rs::PICKUPS_TOPIC`.
/// The relayer subscribes so it appears in clients' mesh views; without
/// this, identified-class flower-pickup messages (M6) have no peer to
/// flow through and propagation silently fails.
const PICKUPS_TOPIC: &str = "roam-pickups/v1";

/// Card-catalog topic. The relayer is the *authority* for which cards
/// exist in its world — different relayer = different catalog =
/// different cards on the ground. Clients subscribe; relayer republishes
/// periodically so freshly-joined peers receive the catalog without
/// any request/response protocol. Must match
/// `roam/src/net/state.rs::CATALOG_TOPIC`.
const CATALOG_TOPIC: &str = "roam-catalog/v1";

/// How often the relayer republishes its catalog. The first publish is
/// at startup; subsequent ticks catch up new peers that joined after
/// the previous publish. 30s is comfortably more often than human
/// reconnect latency, comfortably less often than is wasteful.
const CATALOG_PUBLISH_INTERVAL: Duration = Duration::from_secs(30);

/// Returns the JSON-encoded catalog this relayer publishes. Hardcoded
/// stub of real ccg card slugs for now; the real path is a manifest
/// file generated from `ccg/cards/*.lua` (separate slice) so the
/// relayer's catalog stays in sync with ccg's bundled cards.
///
/// Wire shape matches `roam::catalog::parse_catalog_json`: array of
/// `{"id": "<ccg-slug>", "name": "<display-name>"}` objects.
fn catalog_json() -> String {
    String::from(
        r#"[
            {"id":"amsterdam-city","name":"Amsterdam City"},
            {"id":"anaconda","name":"Anaconda"},
            {"id":"APOPTOSIS","name":"APOPTOSIS"},
            {"id":"amber-dragon","name":"Amber Dragon"},
            {"id":"archer","name":"Archer"},
            {"id":"avatar-of-greed","name":"Avatar of Greed"},
            {"id":"axolotl","name":"Axolotl"},
            {"id":"battle-captain","name":"Battle Captain"}
        ]"#,
    )
}

/// Identify protocol-name advertised on the wire. Must match
/// browser-side rust_libp2p.rs identify config.
const IDENTIFY_PROTOCOL: &str = "/roam/1.0.0";

/// Circuit-relay-v2 reservation cap. Mirrors the TS relay's
/// `circuitRelayServer({ reservations: { maxReservations: 128 } })`.
const MAX_RELAY_RESERVATIONS: usize = 128;

/// Gossipsub heartbeat. The TS relay sets this to 1s explicitly
/// (`heartbeatInterval: 1000`); the browser-worker matches at
/// `Duration::from_secs(1)`. Keep all three the same.
const GOSSIPSUB_HEARTBEAT: Duration = Duration::from_secs(1);

/// CloudWatch metric publishing interval. Re-exports the
/// metrics-module constant so the call-site reads cleanly; both
/// always agree because the same value is referenced.
const CLOUDWATCH_PUBLISH_INTERVAL: Duration = metrics::INTERVAL_DEFAULT;

/// How many minute-cadence samples the status page's sparklines
/// retain. 60 × 60s = 1h of in-process history, byte-cheap to keep
/// and renders in a single SVG polyline per metric.
const STATS_HISTORY_LEN: usize = 60;

/// In-process operational state the status page reads from. Lives
/// behind a `std::sync::Mutex` — critical sections are micro-second
/// scale (push a sample, read counters); no blocking inside async
/// contexts long enough to matter. CloudWatch keeps the
/// authoritative long-history; this is the in-process fast-path
/// snapshot served to every visitor without an AWS round-trip.
struct RelayerStats {
    start: Instant,
    peer_count: u64,
    conn_count: u64,
    total_conns_accepted: u64,
    total_msgs_relayed: u64,
    peer_history: VecDeque<u64>,
    msg_rate_history: VecDeque<f64>,
}

impl RelayerStats {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            peer_count: 0,
            conn_count: 0,
            total_conns_accepted: 0,
            total_msgs_relayed: 0,
            peer_history: VecDeque::with_capacity(STATS_HISTORY_LEN),
            msg_rate_history: VecDeque::with_capacity(STATS_HISTORY_LEN),
        }
    }

    fn push_sample(&mut self, peers: u64, msg_rate: f64) {
        self.peer_count = peers;
        if self.peer_history.len() >= STATS_HISTORY_LEN {
            self.peer_history.pop_front();
        }
        self.peer_history.push_back(peers);
        if self.msg_rate_history.len() >= STATS_HISTORY_LEN {
            self.msg_rate_history.pop_front();
        }
        self.msg_rate_history.push_back(msg_rate);
    }

    fn uptime(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Composite behaviour. NetworkBehaviour derive synthesises the
/// matching event enum. Mirrors the TS relay's `services` block
/// in `relay.ts`.
#[derive(libp2p::swarm::NetworkBehaviour)]
// IDENTITY MENU (roam/docs/IDENTITY.md):
//   M5 — verify gossipsub message signatures here; reject when the claimed
//        source PeerId doesn't match the signing key.
//   D4 — when a status page lands, add a /identity route that exposes
//        this relayer's own libp2p PeerId / did:key.
struct RelayerBehaviour {
    gossipsub: gossipsub::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    relay: relay::Behaviour,
}

#[tokio::main]
async fn main() -> Result<()> {
    // RUST_LOG=info,relayers=debug is a sensible default. The
    // libp2p crates respect tracing too so individual protocols
    // can be turned up without an env-var dance on the box.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,relayers=debug")),
        )
        .init();

    // Test-mode short-circuit. When `ROAM_RELAY_TEST_RANDOM_IDENTITY=1`
    // the relayer mints a fresh Ed25519 keypair instead of fetching
    // from AWS Secrets Manager. The CloudWatch metric loop is
    // independently gated by `ROAM_RELAY_PUBLISH_METRICS=0`. Together
    // these let the integration test at
    // `roam/tests/m6_via_relayer.rs` spawn the binary without any
    // AWS calls ever happening (the sdk builders are cheap and never
    // touch the network until used). The libp2p protocol — the layer
    // whose bugs we care about — is identical between the two modes.
    let test_random_identity = std::env::var("ROAM_RELAY_TEST_RANDOM_IDENTITY")
        .map(|v| v == "1")
        .unwrap_or(false);

    let listen_host =
        std::env::var("ROAM_RELAY_LISTEN_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let listen_port: u16 = std::env::var("ROAM_RELAY_LISTEN_PORT")
        .unwrap_or_else(|_| "9001".to_string())
        .parse()
        .context("ROAM_RELAY_LISTEN_PORT must be a u16")?;
    let announce: Option<Multiaddr> = std::env::var("ROAM_RELAY_ANNOUNCE")
        .ok()
        .map(|s| s.parse())
        .transpose()
        .context("ROAM_RELAY_ANNOUNCE must be a valid multiaddr if set")?;

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let secrets = aws_sdk_secretsmanager::Client::new(&aws_config);
    let cloudwatch = aws_sdk_cloudwatch::Client::new(&aws_config);

    let keypair = if test_random_identity {
        info!("ROAM_RELAY_TEST_RANDOM_IDENTITY=1 — minting random keypair, skipping Secrets Manager fetch");
        identity::Keypair::generate_ed25519()
    } else {
        let identity_secret_arn = std::env::var("ROAM_RELAY_IDENTITY_SECRET")
            .context("ROAM_RELAY_IDENTITY_SECRET must be set to the Secrets Manager ARN")?;
        load_identity(&secrets, &identity_secret_arn).await?
    };
    let local_peer_id = PeerId::from(keypair.public());
    info!(peer_id = %local_peer_id, "identity loaded");

    let mut swarm: Swarm<RelayerBehaviour> = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_other_transport(|key| {
            // WebSocket(noise+yamux). Plain WS — TLS terminates
            // at CloudFront on the deployed box.
            websocket::Config::new(libp2p::tcp::tokio::Transport::new(
                libp2p::tcp::Config::default(),
            ))
            .upgrade(libp2p::core::upgrade::Version::V1)
            .authenticate(noise::Config::new(key).expect("noise config"))
            .multiplex(yamux::Config::default())
        })?
        .with_behaviour(|key| {
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(GOSSIPSUB_HEARTBEAT)
                .validation_mode(gossipsub::ValidationMode::Strict)
                .build()
                .expect("gossipsub config");
            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config,
            )
            .expect("gossipsub behaviour");

            let identify = identify::Behaviour::new(identify::Config::new(
                IDENTIFY_PROTOCOL.to_string(),
                key.public(),
            ));

            let ping = ping::Behaviour::new(ping::Config::new());

            let relay = relay::Behaviour::new(
                key.public().to_peer_id(),
                relay::Config {
                    max_reservations: MAX_RELAY_RESERVATIONS,
                    ..Default::default()
                },
            );

            RelayerBehaviour {
                gossipsub,
                identify,
                ping,
                relay,
            }
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    swarm.behaviour_mut().gossipsub.subscribe(
        &gossipsub::IdentTopic::new(PICKUPS_TOPIC),
    )?;
    info!(topic = PICKUPS_TOPIC, "subscribed");
    swarm.behaviour_mut().gossipsub.subscribe(
        &gossipsub::IdentTopic::new(POSITIONS_TOPIC),
    )?;
    info!(topic = POSITIONS_TOPIC, "subscribed");
    swarm.behaviour_mut().gossipsub.subscribe(
        &gossipsub::IdentTopic::new(CATALOG_TOPIC),
    )?;
    info!(topic = CATALOG_TOPIC, "subscribed");

    // libp2p binds to loopback on an internal port. The public
    // `listen_port` is owned by a tiny TCP front (`status_proxy`
    // below) that either tunnels WebSocket upgrades to libp2p or
    // returns a status HTML for plain HTTP visitors. This is the
    // parity replacement for `bun-ws-transport.ts:135-139`, which
    // returned `426 Upgrade Required` to non-WS GETs; here we go
    // one step further and surface a tiny info page.
    // libp2p loopback port — defaults to 9002 (production), overridable
    // via env so the integration test can pick a free port and avoid
    // clashing with a running prod-shape relayer.
    let libp2p_loopback_port: u16 = std::env::var("ROAM_RELAY_LIBP2P_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9002);
    let libp2p_listen: Multiaddr = format!("/ip4/127.0.0.1/tcp/{libp2p_loopback_port}/ws")
        .parse()
        .context("libp2p loopback multiaddr")?;
    swarm.listen_on(libp2p_listen.clone())?;
    info!(addr = %libp2p_listen, "libp2p listening on loopback");

    let stats = Arc::new(Mutex::new(RelayerStats::new()));

    tokio::spawn(status_proxy(
        listen_host.clone(),
        listen_port,
        libp2p_loopback_port,
        local_peer_id.to_string(),
        stats.clone(),
    ));

    if let Some(announce) = announce {
        swarm.add_external_address(announce.clone());
        info!(addr = %announce, "external address announced");
    }

    let mut cw_interval = tokio::time::interval(CLOUDWATCH_PUBLISH_INTERVAL);
    cw_interval.tick().await; // skip the immediate first tick

    // Catalog interval: publishes at startup (the immediate first
    // `tick()` returns instantly) and every `CATALOG_PUBLISH_INTERVAL`
    // thereafter so newly-joined peers catch up.
    let mut catalog_interval = tokio::time::interval(CATALOG_PUBLISH_INTERVAL);
    let catalog_payload: Vec<u8> = catalog_json().into_bytes();
    info!(
        topic = CATALOG_TOPIC,
        bytes = catalog_payload.len(),
        "catalog payload prepared"
    );

    let metric_namespace = std::env::var("ROAM_RELAY_METRIC_NAMESPACE")
        .unwrap_or_else(|_| metrics::NAMESPACE_DEFAULT.to_string());
    let instance_name = std::env::var("ROAM_RELAY_INSTANCE_NAME")
        .unwrap_or_else(|_| metrics::INSTANCE_DEFAULT.to_string());
    let memory_cap_mb: u64 = std::env::var("ROAM_RELAY_MEMORY_CAP_MB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(metrics::MEMORY_CAP_MB_DEFAULT);
    let publish_metrics = std::env::var("ROAM_RELAY_PUBLISH_METRICS")
        .map(|v| v != "0")
        .unwrap_or(true);

    let mut last_total_msgs: u64 = 0;
    // CloudWatch error tracking — surface once on first failure,
    // then every 60th failure (matches the deleted TS relay's backoff).
    let mut cw_consecutive_errors: u64 = 0;

    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                handle_event(event, &stats);
            }
            _ = catalog_interval.tick() => {
                let topic = gossipsub::IdentTopic::new(CATALOG_TOPIC);
                match swarm.behaviour_mut().gossipsub.publish(topic, catalog_payload.clone()) {
                    Ok(msg_id) => {
                        info!(topic = CATALOG_TOPIC, msg_id = ?msg_id, "catalog published");
                    }
                    Err(gossipsub::PublishError::NoPeersSubscribedToTopic) | Err(gossipsub::PublishError::AllQueuesFull(_)) => {
                        // No subscribed peers / no queue capacity — common
                        // before any clients connect. Downgrade to debug so
                        // restart doesn't spam the journal.
                        tracing::debug!(topic = CATALOG_TOPIC, "catalog publish: no peers / full");
                    }
                    Err(err) => {
                        warn!(topic = CATALOG_TOPIC, err = ?err, "catalog publish failed");
                    }
                }
            }
            _ = cw_interval.tick(), if publish_metrics => {
                let interval_secs = CLOUDWATCH_PUBLISH_INTERVAL.as_secs_f64();
                let peer_count = swarm.connected_peers().count() as u64;
                let (rss, vms) = metrics::read_proc_memory();

                let (rate, conn_count) = {
                    let mut s = stats.lock().expect("stats mutex");
                    let msgs_since_last = s.total_msgs_relayed - last_total_msgs;
                    last_total_msgs = s.total_msgs_relayed;
                    let rate = (msgs_since_last as f64) / interval_secs;
                    s.push_sample(peer_count, rate);
                    (rate, s.conn_count)
                };

                let snapshot = metrics::Snapshot {
                    peers: peer_count,
                    conns: conn_count,
                    pubsub_msgs_per_sec: rate,
                    mem_rss_bytes: rss,
                    mem_vms_bytes: vms,
                };
                let data = metrics::build_metric_data(&snapshot, &instance_name, memory_cap_mb);

                match cloudwatch
                    .put_metric_data()
                    .namespace(&metric_namespace)
                    .set_metric_data(Some(data))
                    .send()
                    .await
                {
                    Ok(_) => {
                        cw_consecutive_errors = 0;
                    }
                    Err(e) => {
                        cw_consecutive_errors += 1;
                        // Surface once on first failure, then every
                        // 60th so the journal doesn't fill with the
                        // same error every minute. Matches relay.ts:328.
                        if cw_consecutive_errors == 1 || cw_consecutive_errors.is_multiple_of(60) {
                            warn!(
                                consecutive = cw_consecutive_errors,
                                error = %e,
                                "cloudwatch PutMetricData failed"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Load the Ed25519 keypair from Secrets Manager. The secret
/// value is expected to be the libp2p canonical protobuf-
/// encoded keypair. If decode fails, we surface the failure as
/// a hard error — silently generating a fresh identity would
/// change the PeerId browsers know, breaking the bootstrap
/// contract.
async fn load_identity(
    client: &aws_sdk_secretsmanager::Client,
    arn: &str,
) -> Result<identity::Keypair> {
    let resp = client
        .get_secret_value()
        .secret_id(arn)
        .send()
        .await
        .with_context(|| format!("get_secret_value({arn})"))?;

    // Q3 verified 2026-06-19: the deployed secret is stored as
    // SecretString containing base64-encoded canonical libp2p
    // protobuf (header `08 01 12 40` = KeyType=Ed25519 + Data
    // length 64). Base64-decode it before handing to
    // `decode_identity`. If a future secret uses SecretBinary
    // (raw bytes, no base64 wrapper), the second branch handles
    // it directly. Both paths refuse to fall through to a fresh
    // keypair — that would change the PeerId browsers know.
    let bytes = if let Some(s) = resp.secret_string() {
        identity_secret::decode_base64(s)
            .context("SecretString is not valid base64 — see relayers.md Q2")?
    } else if let Some(b) = resp.secret_binary() {
        b.as_ref().to_vec()
    } else {
        anyhow::bail!("secret has neither SecretString nor SecretBinary value")
    };

    identity_secret::decode_identity(&bytes)
        .context("secret is not a libp2p canonical protobuf keypair — see relayers.md Q2")
}

/// Log connection lifecycle and behaviour events. Mirrors the TS
/// relay's `[relay] peer:connect / peer:disconnect /
/// connection:open / connection:close` logging and increments
/// the per-interval counters the CloudWatch publisher reads.
fn handle_event(
    event: SwarmEvent<RelayerBehaviourEvent>,
    stats: &Arc<Mutex<RelayerStats>>,
) {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            info!(addr = %address, "new listen address");
        }
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => {
            let conns = {
                let mut s = stats.lock().expect("stats mutex");
                s.conn_count = s.conn_count.saturating_add(1);
                s.total_conns_accepted = s.total_conns_accepted.saturating_add(1);
                s.conn_count
            };
            info!(peer = %peer_id, ?endpoint, conns = conns, "connection:open");
        }
        SwarmEvent::ConnectionClosed {
            peer_id, cause, ..
        } => {
            let conns = {
                let mut s = stats.lock().expect("stats mutex");
                s.conn_count = s.conn_count.saturating_sub(1);
                s.conn_count
            };
            info!(peer = %peer_id, cause = ?cause, conns = conns, "connection:close");
        }
        SwarmEvent::IncomingConnectionError { error, .. } => {
            warn!(error = ?error, "incoming connection error");
        }
        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            warn!(peer = ?peer_id, error = ?error, "outgoing connection error");
        }
        SwarmEvent::Behaviour(RelayerBehaviourEvent::Gossipsub(
            gossipsub::Event::Message { .. },
        )) => {
            // Counted, not logged — at 20Hz this would drown the
            // journal. The interval rate ends up in CloudWatch
            // (`relay_pubsub_msgs_per_sec`) which is the load-
            // bearing observation; the per-message granularity is
            // not useful for the relayer's operational view.
            let mut s = stats.lock().expect("stats mutex");
            s.total_msgs_relayed = s.total_msgs_relayed.saturating_add(1);
        }
        SwarmEvent::Behaviour(RelayerBehaviourEvent::Gossipsub(
            gossipsub::Event::Subscribed { peer_id, topic },
        )) => {
            info!(peer = %peer_id, topic = %topic, "subscription-change +");
        }
        SwarmEvent::Behaviour(RelayerBehaviourEvent::Gossipsub(
            gossipsub::Event::Unsubscribed { peer_id, topic },
        )) => {
            info!(peer = %peer_id, topic = %topic, "subscription-change -");
        }
        _ => {}
    }
}

// CloudWatch publish is inlined into the main select_loop above
// — see the `cw_interval.tick()` branch. The metric_data shape
// lives in `metrics.rs` (pure function, tested); the AWS SDK
// call shape lives at the call site so the per-attempt error
// path (consecutive-error backoff) stays adjacent to the loop
// state it reads.

/// TCP front on the public port. Each incoming connection gets one
/// chance to look like a WebSocket upgrade (peek the first chunk,
/// scan for `Upgrade: websocket`). If yes, splice bytes to the
/// libp2p loopback listener (which speaks soketto's WS protocol);
/// if no, write a 200 + status HTML + close. CloudFront's WSS path
/// goes through the WS branch; a casual browser visitor to
/// `https://relay.sbvh.nl/` gets the HTML.
async fn status_proxy(
    host: String,
    public_port: u16,
    libp2p_port: u16,
    peer_id: String,
    stats: Arc<Mutex<RelayerStats>>,
) {
    let bind_addr = format!("{host}:{public_port}");
    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, addr = %bind_addr, "status_proxy bind failed");
            return;
        }
    };
    info!(addr = %bind_addr, libp2p_port, "status proxy listening");
    loop {
        let (socket, _peer) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "status_proxy accept error");
                continue;
            }
        };
        let peer_id = peer_id.clone();
        let stats = stats.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_status_conn(socket, libp2p_port, &peer_id, &stats).await
            {
                tracing::debug!(error = %e, "status_proxy connection ended with error");
            }
        });
    }
}

async fn handle_status_conn(
    mut socket: tokio::net::TcpStream,
    libp2p_port: u16,
    peer_id: &str,
    stats: &Arc<Mutex<RelayerStats>>,
) -> std::io::Result<()> {
    let mut peek_buf = vec![0u8; 8192];
    let n = socket.peek(&mut peek_buf).await?;
    if n == 0 {
        return Ok(());
    }
    if looks_like_websocket_upgrade(&peek_buf[..n]) {
        let mut upstream =
            tokio::net::TcpStream::connect(("127.0.0.1", libp2p_port)).await?;
        tokio::io::copy_bidirectional(&mut socket, &mut upstream).await?;
    } else {
        use tokio::io::AsyncWriteExt;
        // Hold the lock only long enough to clone snapshot data; the
        // HTML render itself runs lock-free.
        let snapshot = {
            let s = stats.lock().expect("stats mutex");
            StatsSnapshot {
                uptime: s.uptime(),
                peer_count: s.peer_count,
                conn_count: s.conn_count,
                total_conns_accepted: s.total_conns_accepted,
                total_msgs_relayed: s.total_msgs_relayed,
                peer_history: s.peer_history.iter().copied().collect(),
                msg_rate_history: s.msg_rate_history.iter().copied().collect(),
            }
        };
        let body = build_status_html(peer_id, &snapshot);
        let response = format_status_response(&body);
        socket.write_all(&response).await?;
        socket.shutdown().await?;
    }
    Ok(())
}

/// Cheap-to-clone snapshot used by `handle_status_conn`. The lock on
/// `RelayerStats` is released the moment this is built; the SVG +
/// HTML render runs against owned data.
struct StatsSnapshot {
    uptime: Duration,
    peer_count: u64,
    conn_count: u64,
    total_conns_accepted: u64,
    total_msgs_relayed: u64,
    peer_history: Vec<u64>,
    msg_rate_history: Vec<f64>,
}

/// Case-insensitive scan for `Upgrade: websocket` in the request
/// preview. Browsers and CloudFront both send this exact header
/// when initiating a WS handshake; absence = plain HTTP visitor.
fn looks_like_websocket_upgrade(bytes: &[u8]) -> bool {
    let needle = b"upgrade: websocket";
    let lower: Vec<u8> = bytes.iter().map(|b| b.to_ascii_lowercase()).collect();
    lower.windows(needle.len()).any(|w| w == needle)
}

/// Build the public status HTML. Pure function — takes the peer-id
/// and snapshot, returns markup. CloudFront fronts this; sparklines
/// move at 60s cadence so a CloudFront default TTL of a few seconds
/// is already fresh enough without an explicit Cache-Control.
fn build_status_html(peer_id: &str, snap: &StatsSnapshot) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let uptime = format_uptime(snap.uptime);
    let peers_spark = render_sparkline_u64(&snap.peer_history);
    let msgs_spark = render_sparkline_f64(&snap.msg_rate_history);
    let msg_rate_now = snap
        .msg_rate_history
        .last()
        .copied()
        .unwrap_or(0.0);
    format!(
        "<!doctype html><html lang=en><head><meta charset=utf-8>\
<title>roam relay</title>\
<style>body{{font:14px/1.5 ui-monospace,Menlo,monospace;max-width:42em;\
margin:3em auto;padding:0 1em;color:#ddd;background:#111}}\
a{{color:#6cf}}code{{background:#222;padding:0 .3em}}\
h1{{font-size:1.2em;margin:0 0 1em}}p{{margin:.5em 0}}\
table{{border-collapse:collapse;margin:1em 0;width:100%}}\
th,td{{padding:.3em .6em;border-bottom:1px solid #222;text-align:left}}\
th{{color:#9ad;font-weight:normal;width:14em}}\
.spark{{height:1.6em;vertical-align:middle}}\
.spark path{{fill:none;stroke:#6cf;stroke-width:1.4}}\
.spark .bg{{fill:#1a1a1a;stroke:none}}\
.muted{{color:#888;font-size:.9em}}</style></head><body>\
<h1>roam relay</h1>\
<p>libp2p relayer for <a href=\"https://roam.sbvh.nl/\">roam.sbvh.nl</a>.</p>\
<p>WebSocket: <code>wss://relay.sbvh.nl/</code></p>\
<table>\
<tr><th>PeerId</th><td><code>{peer_id}</code></td></tr>\
<tr><th>Version</th><td>relayers {version}</td></tr>\
<tr><th>Uptime</th><td>{uptime}</td></tr>\
<tr><th>Connected peers</th><td>{peers_now} (open conns: {conns_now})</td></tr>\
<tr><th>Peers (1h)</th><td>{peers_spark}</td></tr>\
<tr><th>Pubsub msgs/s</th><td>{rate_now:.2}</td></tr>\
<tr><th>Pubsub rate (1h)</th><td>{msgs_spark}</td></tr>\
<tr><th>Connections accepted</th><td>{total_conns}</td></tr>\
<tr><th>Messages relayed</th><td>{total_msgs}</td></tr>\
</table>\
<p class=muted>Sparklines cover the last hour at 60s cadence. \
Authoritative history (with alarms) lives in CloudWatch namespace \
<code>CWAgent</code>, dimension <code>InstanceName=roam-relay-eu-2</code>.</p>\
<p>Source: <a href=\"https://github.com/teranos/tsot/tree/master/roam/relayers\">\
github.com/teranos/tsot/roam/relayers</a></p>\
</body></html>",
        peer_id = peer_id,
        version = version,
        uptime = uptime,
        peers_now = snap.peer_count,
        conns_now = snap.conn_count,
        peers_spark = peers_spark,
        rate_now = msg_rate_now,
        msgs_spark = msgs_spark,
        total_conns = snap.total_conns_accepted,
        total_msgs = snap.total_msgs_relayed,
    )
}

/// Format a `Duration` as `2d 03h 14m` style. Days roll up; sub-
/// minute is just "<1m" — a relayer that hasn't been up a minute
/// isn't interesting to characterize precisely.
fn format_uptime(d: Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs < 60 {
        return "<1m".into();
    }
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let mins = (total_secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours:02}h {mins:02}m")
    } else if hours > 0 {
        format!("{hours}h {mins:02}m")
    } else {
        format!("{mins}m")
    }
}

/// SVG polyline sparkline over `u64` samples. Empty input renders an
/// empty rect with a "no data" label so the page layout stays stable
/// during the first 60s.
fn render_sparkline_u64(samples: &[u64]) -> String {
    let as_f64: Vec<f64> = samples.iter().map(|&v| v as f64).collect();
    render_sparkline_f64(&as_f64)
}

fn render_sparkline_f64(samples: &[f64]) -> String {
    let width = 240.0_f64;
    let height = 24.0_f64;
    if samples.is_empty() {
        return format!(
            "<svg class=spark viewBox=\"0 0 {w} {h}\" width=\"{w}\" \
height=\"{h}\" preserveAspectRatio=\"none\">\
<rect class=bg width=\"{w}\" height=\"{h}\"/>\
<text x=\"{tx}\" y=\"{ty}\" fill=\"#666\" font-size=\"10\" \
text-anchor=\"middle\">no data yet</text></svg>",
            w = width,
            h = height,
            tx = width / 2.0,
            ty = height / 2.0 + 3.0,
        );
    }
    let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = (max - min).max(1e-9);
    // X step assumes the buffer fills the full width even when partially
    // populated — this is "Cloudflare-style" right-anchored: 60 slots,
    // unfilled slots at the left are blank, line starts where data starts.
    let slots = STATS_HISTORY_LEN.max(samples.len()) as f64;
    let step = width / (slots - 1.0).max(1.0);
    let leading_blank = (slots as usize).saturating_sub(samples.len());
    let mut d = String::new();
    for (i, v) in samples.iter().enumerate() {
        let x = (leading_blank + i) as f64 * step;
        // Invert Y so larger samples sit higher; pad 2px top/bottom.
        let y = if max <= min {
            height / 2.0
        } else {
            2.0 + (height - 4.0) * (1.0 - (v - min) / range)
        };
        if i == 0 {
            d.push_str(&format!("M{x:.1},{y:.1}"));
        } else {
            d.push_str(&format!(" L{x:.1},{y:.1}"));
        }
    }
    format!(
        "<svg class=spark viewBox=\"0 0 {w} {h}\" width=\"{w}\" \
height=\"{h}\" preserveAspectRatio=\"none\">\
<rect class=bg width=\"{w}\" height=\"{h}\"/>\
<path d=\"{d}\"/></svg>",
        w = width,
        h = height,
        d = d,
    )
}

fn format_status_response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
Content-Length: {}\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes()
}

#[cfg(test)]
mod proxy_tests {
    use super::*;

    /// A real Firefox WSS handshake request fragment. The proxy
    /// must classify this as a WebSocket upgrade so libp2p sees
    /// the bytes via the loopback splice.
    #[test]
    fn classifies_firefox_ws_upgrade() {
        let req = b"GET / HTTP/1.1\r\nHost: relay.sbvh.nl\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: x\r\n\r\n";
        assert!(looks_like_websocket_upgrade(req));
    }

    /// Header capitalization varies across clients (CloudFront
    /// often lowercases). Must still classify as upgrade.
    #[test]
    fn classifies_lowercase_upgrade_header() {
        let req = b"get / http/1.1\r\nhost: relay.sbvh.nl\r\nupgrade: websocket\r\n\r\n";
        assert!(looks_like_websocket_upgrade(req));
    }

    /// Plain `curl https://relay.sbvh.nl/` — no Upgrade header.
    /// Must NOT classify as upgrade (the visitor gets the status HTML).
    #[test]
    fn rejects_plain_http_get() {
        let req = b"GET / HTTP/1.1\r\nHost: relay.sbvh.nl\r\nUser-Agent: curl/8\r\n\r\n";
        assert!(!looks_like_websocket_upgrade(req));
    }

    fn fake_snapshot() -> StatsSnapshot {
        StatsSnapshot {
            uptime: Duration::from_secs(3725), // 1h 02m 05s
            peer_count: 4,
            conn_count: 5,
            total_conns_accepted: 42,
            total_msgs_relayed: 9001,
            peer_history: vec![1, 2, 3, 4],
            msg_rate_history: vec![0.5, 1.25, 2.0, 1.0],
        }
    }

    /// Status HTML must contain the PeerId so a visitor can
    /// verify which relayer they're looking at, plus the
    /// SRE-style stats that motivate this view.
    #[test]
    fn status_html_embeds_peer_id_and_stats() {
        let html = build_status_html("12D3KooWtestPeer", &fake_snapshot());
        assert!(html.contains("12D3KooWtestPeer"));
        assert!(html.contains("wss://relay.sbvh.nl/"));
        assert!(html.contains("github.com/teranos/tsot"));
        // Counters surface.
        assert!(html.contains("42"), "total_conns_accepted in html");
        assert!(html.contains("9001"), "total_msgs_relayed in html");
        // Sparkline SVG markup is inline (no external resources).
        assert!(html.contains("<svg class=spark"));
        // Authoritative-store breadcrumb so operators know the page is
        // a fast-path snapshot, not the source of truth.
        assert!(html.contains("CWAgent"));
    }

    /// Empty history must render a placeholder sparkline so the
    /// first-60s page state still lays out correctly.
    #[test]
    fn empty_history_renders_no_data_sparkline() {
        let svg = render_sparkline_f64(&[]);
        assert!(svg.contains("no data yet"));
        assert!(svg.contains("<svg"));
    }

    /// Sparkline path command count matches sample count (one M, N-1 L's).
    /// Locks the "Cloudflare-style right-anchored line" shape so a future
    /// refactor doesn't silently drop samples.
    #[test]
    fn sparkline_path_has_one_move_and_rest_lines() {
        let svg = render_sparkline_f64(&[1.0, 2.0, 3.0, 4.0]);
        let move_count = svg.matches('M').count();
        let line_count = svg.matches(" L").count();
        assert_eq!(move_count, 1, "single M command");
        assert_eq!(line_count, 3, "N-1 L commands for N samples");
    }

    /// All-equal samples must not produce NaN/inf paths. Edge case —
    /// new relayer with peer_count flat at 0 was producing div-by-zero
    /// before the `range.max(1e-9)` guard.
    #[test]
    fn flat_samples_render_without_nan() {
        let svg = render_sparkline_f64(&[0.0, 0.0, 0.0]);
        assert!(!svg.contains("NaN"));
        assert!(!svg.contains("inf"));
    }

    #[test]
    fn uptime_under_one_minute_is_terse() {
        assert_eq!(format_uptime(Duration::from_secs(30)), "<1m");
    }

    #[test]
    fn uptime_formats_minutes_hours_days() {
        assert_eq!(format_uptime(Duration::from_secs(60 * 5)), "5m");
        assert_eq!(format_uptime(Duration::from_secs(60 * 65)), "1h 05m");
        assert_eq!(
            format_uptime(Duration::from_secs(86_400 * 2 + 3_600 * 3 + 60 * 14)),
            "2d 03h 14m"
        );
    }

    /// Ring buffer push must evict oldest beyond the cap. Encodes
    /// the "60 samples = 1h at 60s cadence" invariant the sparklines
    /// implicitly assume.
    #[test]
    fn stats_history_evicts_at_cap() {
        let mut s = RelayerStats::new();
        for i in 0..(STATS_HISTORY_LEN + 10) {
            s.push_sample(i as u64, i as f64);
        }
        assert_eq!(s.peer_history.len(), STATS_HISTORY_LEN);
        assert_eq!(s.msg_rate_history.len(), STATS_HISTORY_LEN);
        // Oldest 10 samples evicted; first remaining is index 10.
        assert_eq!(*s.peer_history.front().expect("non-empty"), 10);
    }

    /// HTTP response framing must declare Content-Length matching
    /// the body length and close the connection (no keep-alive
    /// because the front is single-shot per connection).
    #[test]
    fn http_response_framing() {
        let body = "x".repeat(123);
        let resp = format_status_response(&body);
        let resp_str = std::str::from_utf8(&resp).unwrap();
        assert!(resp_str.starts_with("HTTP/1.1 200 OK"));
        assert!(resp_str.contains("Content-Length: 123"));
        assert!(resp_str.contains("Connection: close"));
        assert!(resp_str.ends_with(&body));
    }

    /// CloudFront caches the response by default-TTL when the origin
    /// emits no Cache-Control — observed during 0.3.1 deploy as a
    /// stale page after a fresh build. max-age=60 matches the
    /// sparkline sample cadence: a visitor sees something that's at
    /// most one sample stale, never older.
    #[test]
    fn status_response_emits_cache_control() {
        let resp = format_status_response("body");
        let resp_str = std::str::from_utf8(&resp).unwrap();
        assert!(
            resp_str.contains("Cache-Control: max-age=60"),
            "must emit Cache-Control: max-age=60 (got: {resp_str})"
        );
    }
}
