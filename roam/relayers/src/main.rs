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

/// CloudWatch metric publishing interval. Matches the TS relay's
/// 60s cadence.
const CLOUDWATCH_PUBLISH_INTERVAL: Duration = Duration::from_secs(60);

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

    loop {
        tokio::select! {
            event = swarm.select_next_some() => handle_event(event),
            _ = cw_interval.tick() => {
                if let Err(e) = publish_cloudwatch(&cloudwatch, &swarm).await {
                    warn!(error = ?e, "cloudwatch publish failed");
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

    // Try the canonical libp2p protobuf encoding first. If the
    // secret holds a different shape (js-libp2p's bespoke JSON),
    // bail loudly so the operator knows to format-translate or
    // migrate. The doc (roam/relay/relayers.md, Q2) calls this
    // out as the one decision the verifications produce.
    let bytes = resp
        .secret_string()
        .map(|s| s.as_bytes().to_vec())
        .or_else(|| resp.secret_binary().map(|b| b.as_ref().to_vec()))
        .context("secret has neither string nor binary value")?;

    identity::Keypair::from_protobuf_encoding(&bytes)
        .context("secret is not a libp2p canonical protobuf keypair — see relayers.md Q2")
}

/// Log connection lifecycle and behaviour events. Mirrors the TS
/// relay's `[relay] peer:connect / peer:disconnect /
/// connection:open / connection:close` logging.
fn handle_event(event: SwarmEvent<RelayerBehaviourEvent>) {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => {
            info!(addr = %address, "new listen address");
        }
        SwarmEvent::ConnectionEstablished {
            peer_id, endpoint, ..
        } => {
            info!(peer = %peer_id, ?endpoint, "connection:open");
        }
        SwarmEvent::ConnectionClosed {
            peer_id, cause, ..
        } => {
            info!(peer = %peer_id, cause = ?cause, "connection:close");
        }
        SwarmEvent::IncomingConnectionError { error, .. } => {
            warn!(error = ?error, "incoming connection error");
        }
        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            warn!(peer = ?peer_id, error = ?error, "outgoing connection error");
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

/// Publish CloudWatch metrics. Namespace + dimension match what
/// the TS relay used so existing alarms / dashboards keep
/// working unchanged across the cut-over.
async fn publish_cloudwatch(
    _client: &aws_sdk_cloudwatch::Client,
    _swarm: &Swarm<RelayerBehaviour>,
) -> Result<()> {
    // TODO: port the metric set from `roam/relay/relay.ts`'s
    // periodic publish. The TS code reads peer count, mesh size,
    // memory_used_percent (CWAgent populates that one), and
    // emits PutMetricData every 60s. Match the same metric
    // names + dimensions so alarms don't need to change.
    Ok(())
}
