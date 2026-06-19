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

use std::time::Duration;

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

/// Composite behaviour. NetworkBehaviour derive synthesises the
/// matching event enum. Mirrors the TS relay's `services` block
/// in `relay.ts`.
#[derive(libp2p::swarm::NetworkBehaviour)]
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

    let identity_secret_arn = std::env::var("ROAM_RELAY_IDENTITY_SECRET")
        .context("ROAM_RELAY_IDENTITY_SECRET must be set to the Secrets Manager ARN")?;
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

    let keypair = load_identity(&secrets, &identity_secret_arn).await?;
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
        &gossipsub::IdentTopic::new(POSITIONS_TOPIC),
    )?;
    info!(topic = POSITIONS_TOPIC, "subscribed");

    let listen_addr: Multiaddr = format!("/ip4/{listen_host}/tcp/{listen_port}/ws")
        .parse()
        .context("listen multiaddr")?;
    swarm.listen_on(listen_addr.clone())?;
    info!(addr = %listen_addr, "listening");

    if let Some(announce) = announce {
        swarm.add_external_address(announce.clone());
        info!(addr = %announce, "external address announced");
    }

    let mut cw_interval = tokio::time::interval(CLOUDWATCH_PUBLISH_INTERVAL);
    cw_interval.tick().await; // skip the immediate first tick

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

    // Counters for the per-interval rate metric and for the
    // peer/conn snapshots we publish to CloudWatch. Updated by
    // `handle_event` on each SwarmEvent.
    let mut pubsub_msg_count: u64 = 0;
    let mut last_pubsub_msg_count: u64 = 0;
    let mut conn_count: u64 = 0;
    // CloudWatch error tracking — surface once on first failure,
    // then every 60th failure (matches relay.ts:328 backoff).
    let mut cw_consecutive_errors: u64 = 0;

    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                handle_event(event, &mut pubsub_msg_count, &mut conn_count);
            }
            _ = cw_interval.tick(), if publish_metrics => {
                let interval_secs = CLOUDWATCH_PUBLISH_INTERVAL.as_secs_f64();
                let msgs_since_last = pubsub_msg_count - last_pubsub_msg_count;
                last_pubsub_msg_count = pubsub_msg_count;
                let rate = (msgs_since_last as f64) / interval_secs;
                let (rss, vms) = metrics::read_proc_memory();
                let peer_count = swarm.connected_peers().count() as u64;

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
    pubsub_msg_count: &mut u64,
    conn_count: &mut u64,
) {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            info!(addr = %address, "new listen address");
        }
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => {
            *conn_count = conn_count.saturating_add(1);
            info!(peer = %peer_id, ?endpoint, conns = *conn_count, "connection:open");
        }
        SwarmEvent::ConnectionClosed {
            peer_id, cause, ..
        } => {
            *conn_count = conn_count.saturating_sub(1);
            info!(peer = %peer_id, cause = ?cause, conns = *conn_count, "connection:close");
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
            *pubsub_msg_count = pubsub_msg_count.saturating_add(1);
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
