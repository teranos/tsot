// libp2p publish path for seer's RunSummary (Task 13). Extracted from
// seer-host so the sibling `seer-p2p-test` integration crate can call
// publish_summary directly against a loopback subscriber, in the same
// shape rave-positions-test uses for rave-positions/v1.
//
// After seer-host finishes a diagnostic run, if SEER_P2P_BOOTSTRAP is
// set, we dial the given multiaddress, subscribe to seer-summary/v1,
// publish the summary as a signed gossipsub message, wait briefly for
// propagation, then exit. Best-effort: any failure (no route,
// timeout, gossipsub not meshed) is logged and swallowed by the
// caller so the local diagnostic never blocks on the network.
//
// Reuses the existing `relaye.sbvh.nl` deployment — the relayer only
// needs `seer-summary/v1` added to its RELAYE_TOPICS env for messages
// on this topic to mesh (one line of deploy config).

use anyhow::{Context, Result};
use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    Multiaddr, SwarmBuilder,
    core::{transport::Transport, upgrade},
    gossipsub::{self, IdentTopic, MessageAuthenticity, ValidationMode},
    identify, identity, noise, ping,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    tcp, websocket, yamux,
};
use serde::Serialize;

pub const SEER_SUMMARY_TOPIC: &str = "seer-summary/v1";
pub const IDENTIFY_PROTOCOL: &str = "/seer/1.0.0";

#[derive(NetworkBehaviour)]
pub struct SeerP2PBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
}

/// Build a client swarm with the same behaviours the rave positions
/// test uses (gossipsub + identify + ping over websocket-on-tcp).
/// Kept public so the integration test can spawn a mirror-image
/// subscriber swarm on loopback.
pub fn build_swarm() -> Result<Swarm<SeerP2PBehaviour>> {
    let keypair = identity::Keypair::generate_ed25519();
    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_other_transport(|key| {
            websocket::Config::new(tcp::tokio::Transport::new(tcp::Config::default()))
                .upgrade(upgrade::Version::V1)
                .authenticate(noise::Config::new(key).expect("noise config"))
                .multiplex(yamux::Config::default())
        })
        .map_err(|e| anyhow::anyhow!("transport builder: {e}"))?
        .with_behaviour(|key| {
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .validation_mode(ValidationMode::Strict)
                .build()
                .expect("gossipsub config");
            let gossipsub = gossipsub::Behaviour::new(
                MessageAuthenticity::Signed(key.clone()),
                gossipsub_config,
            )
            .expect("gossipsub behaviour");
            let identify = identify::Behaviour::new(identify::Config::new(
                IDENTIFY_PROTOCOL.to_string(),
                key.public(),
            ));
            let ping = ping::Behaviour::new(ping::Config::default());
            SeerP2PBehaviour {
                gossipsub,
                identify,
                ping,
            }
        })
        .map_err(|e| anyhow::anyhow!("behaviour: {e}"))?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(30)))
        .build();
    Ok(swarm)
}

/// Publish `payload` (any Serialize) to the seer-summary/v1 topic on
/// the swarm reachable through `bootstrap`. Deadline caps the whole
/// dance so CI never stalls on a slow / unreachable relayer.
pub async fn publish_summary<T: Serialize>(
    payload: &T,
    bootstrap: &str,
    deadline: Duration,
) -> Result<()> {
    let addr: Multiaddr = bootstrap
        .parse()
        .with_context(|| format!("invalid bootstrap multiaddr: {bootstrap}"))?;

    let mut swarm = build_swarm()?;
    let topic = IdentTopic::new(SEER_SUMMARY_TOPIC);
    swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&topic)
        .with_context(|| format!("subscribe {SEER_SUMMARY_TOPIC}"))?;
    swarm
        .dial(addr.clone())
        .with_context(|| format!("dial {addr}"))?;

    let bytes = serde_json::to_vec(payload).context("serialize gossipsub payload")?;

    let sleep = tokio::time::sleep(deadline);
    tokio::pin!(sleep);

    let mut mesh_ready = false;
    let mut published = false;

    loop {
        tokio::select! {
            _ = &mut sleep => {
                return Err(anyhow::anyhow!(
                    "publish deadline hit (mesh_ready={mesh_ready} published={published})"
                ));
            }
            ev = swarm.select_next_some() => match ev {
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("[p2p] connected peer {peer_id}");
                }
                SwarmEvent::Behaviour(SeerP2PBehaviourEvent::Gossipsub(
                    gossipsub::Event::Subscribed { peer_id, topic: t }
                )) if t == topic.hash() => {
                    println!("[p2p] peer {peer_id} subscribed {SEER_SUMMARY_TOPIC}");
                    mesh_ready = true;
                    if !published {
                        match swarm
                            .behaviour_mut()
                            .gossipsub
                            .publish(topic.clone(), bytes.clone())
                        {
                            Ok(_) => {
                                println!(
                                    "[p2p] published {} bytes to {SEER_SUMMARY_TOPIC}",
                                    bytes.len()
                                );
                                published = true;
                            }
                            Err(e) => {
                                println!("[p2p] publish failed: {e}");
                            }
                        }
                    }
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    println!("[p2p] outgoing conn error peer={peer_id:?} err={error}");
                }
                _ => {}
            }
        }
        if published {
            // Small tail-wait so gossipsub actually flushes the message
            // over the wire before we drop the swarm. Publisher doesn't
            // receive its own broadcast, so we can't wait on that.
            tokio::time::sleep(Duration::from_millis(500)).await;
            return Ok(());
        }
    }
}
