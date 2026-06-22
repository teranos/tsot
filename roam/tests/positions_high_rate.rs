#![cfg(not(target_arch = "wasm32"))]
//! Real-wire stress test for the positions topic at production publish
//! rate (~5 Hz).
//!
//! Why this exists: in the 0.3.6 session the user surfaced a confusing
//! asymmetry — pickups propagated cleanly end-to-end across two
//! browser tabs (proven by `tests/m6_via_relayer.rs`), but positions
//! continuously failed publish with `NoPeersSubscribedToTopic`
//! (collapse-counts >85 per log row, sustained across minutes). Both
//! topics share the same client subscribe path and the same relayer
//! — the only obvious thing different about positions is the publish
//! rate (~5 Hz, vs pickups' sporadic).
//!
//! This test stands up the same real-wire harness `m6_via_relayer`
//! uses (real relayer binary + two native libp2p client swarms over
//! WebSocket loopback) and runs the production publish cadence on the
//! positions topic for 10 seconds — 50 messages from client A. It
//! asserts that ≥ 90% reach client B (45+ of 50), AND that no publish
//! returned `NoPeersSubscribedToTopic`.
//!
//! Failure modes the test surfaces:
//!   - If publishes return `NoPeersSubscribedToTopic` mid-stream →
//!     the mesh is empty from A's view *at publish time*, even though
//!     the subscribe announcement reached the relayer. That's the
//!     production symptom we're chasing.
//!   - If receive rate is below 90% → mesh is up but messages are
//!     being dropped (peer scoring, message-id collisions, etc.).
//!
//! What helpers are duplicated from `m6_via_relayer.rs`: `pick_free_port`,
//! `build_client_swarm`, and the relayer-spawn + wait-for-ready pattern.
//! Extracting them to a `tests/common/` module is the obvious follow-up
//! once a third real-wire test arrives.

use std::collections::HashSet;
use std::process::Stdio;
use std::time::Duration;

use libp2p::futures::StreamExt;
use libp2p::{
    core::{transport::Transport, upgrade},
    gossipsub::{self, IdentTopic, MessageAuthenticity, PublishError, ValidationMode},
    identify, identity, noise, ping,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    tcp, websocket, yamux, Multiaddr, SwarmBuilder,
};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

const POSITIONS_TOPIC: &str = "roam-positions/v1";
const IDENTIFY_PROTOCOL: &str = "/roam/1.0.0";

#[derive(NetworkBehaviour)]
struct ClientBehaviour {
    gossipsub: gossipsub::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
}

fn build_client_swarm() -> Swarm<ClientBehaviour> {
    let keypair = identity::Keypair::generate_ed25519();
    SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_other_transport(|key| {
            websocket::Config::new(tcp::tokio::Transport::new(tcp::Config::default()))
                .upgrade(upgrade::Version::V1)
                .authenticate(noise::Config::new(key).expect("noise config"))
                .multiplex(yamux::Config::default())
        })
        .expect("transport")
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
            ClientBehaviour {
                gossipsub,
                identify,
                ping,
            }
        })
        .expect("behaviour")
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build()
}

fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 0");
    listener.local_addr().expect("local_addr").port()
}

#[tokio::test(flavor = "multi_thread")]
async fn positions_propagate_at_5hz_for_10s() {
    // === 1. Build the relayer (release). Cached on subsequent runs. ===
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let relayers_dir = std::path::PathBuf::from(manifest_dir).join("relayers");
    let status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&relayers_dir)
        .status()
        .await
        .expect("invoke cargo build");
    assert!(status.success(), "cargo build --release in relayers/ failed");

    // === 2. Pick ports, locate the binary. ===
    let libp2p_port = pick_free_port();
    let status_port = pick_free_port();
    let relayer_bin = relayers_dir
        .join("target")
        .join("release")
        .join("relayers");
    assert!(relayer_bin.exists(), "relayer binary missing at {relayer_bin:?}");

    // === 3. Spawn the relayer in test mode. ===
    let mut relayer_child = Command::new(&relayer_bin)
        .env("ROAM_RELAY_TEST_RANDOM_IDENTITY", "1")
        .env("ROAM_RELAY_LIBP2P_PORT", libp2p_port.to_string())
        .env("ROAM_RELAY_LISTEN_PORT", status_port.to_string())
        .env("ROAM_RELAY_LISTEN_HOST", "127.0.0.1")
        .env("ROAM_RELAY_PUBLISH_METRICS", "0")
        .env("RUST_LOG", "info,relayers=info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn relayer");

    // === 4. Wait for `libp2p listening on loopback` in stdout. ===
    let stdout = relayer_child.stdout.take().expect("stdout");
    let mut reader = tokio::io::BufReader::new(stdout).lines();
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Ok(Some(line)) = reader.next_line().await {
            if line.contains("libp2p listening on loopback") {
                return;
            }
        }
        panic!("relayer stdout closed before listening");
    })
    .await
    .expect("relayer never reported listening");

    // === 5. Build two client swarms, subscribe both to positions, dial. ===
    let mut client_a = build_client_swarm();
    let mut client_b = build_client_swarm();
    let a_peer_id = *client_a.local_peer_id();
    let positions = IdentTopic::new(POSITIONS_TOPIC);
    client_a
        .behaviour_mut()
        .gossipsub
        .subscribe(&positions)
        .expect("A subscribe");
    client_b
        .behaviour_mut()
        .gossipsub
        .subscribe(&positions)
        .expect("B subscribe");
    let relay_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/{libp2p_port}/ws")
        .parse()
        .expect("relay multiaddr");
    client_a.dial(relay_addr.clone()).expect("A dial");
    client_b.dial(relay_addr.clone()).expect("B dial");

    // === 6. Wait until both have seen the relayer's Subscribed event
    // for positions — that's the production-meaningful "mesh ready"
    // signal. ===
    let mut a_meshed = false;
    let mut b_meshed = false;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                ev = client_a.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Subscribed { topic, .. }
                    )) = ev {
                        if topic.as_str() == POSITIONS_TOPIC {
                            a_meshed = true;
                        }
                    }
                }
                ev = client_b.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Subscribed { topic, .. }
                    )) = ev {
                        if topic.as_str() == POSITIONS_TOPIC {
                            b_meshed = true;
                        }
                    }
                }
            }
            if a_meshed && b_meshed {
                return;
            }
        }
    })
    .await
    .expect("mesh did not form within 30s — relayer never propagated positions subscribe");

    // === 7. Drive at production publish cadence for 10 seconds. ===
    // Production roam broadcasts position at ~5 Hz from the bridge's
    // animation timer. 50 publishes over 10s mirrors that, plus a 10s
    // drain window after the publish loop ends for any in-flight
    // messages to settle on B's side.
    const PUBLISH_COUNT: u32 = 50;
    const PUBLISH_INTERVAL_MS: u64 = 200;
    let mut interval = tokio::time::interval(Duration::from_millis(PUBLISH_INTERVAL_MS));
    let mut published_ok: u32 = 0;
    let mut publish_errors: Vec<(u32, String)> = Vec::new();
    let mut received_indices: HashSet<u32> = HashSet::new();

    let publish_deadline = tokio::time::Instant::now() + Duration::from_secs(11);
    let total_deadline = tokio::time::Instant::now() + Duration::from_secs(25);

    loop {
        let now = tokio::time::Instant::now();
        if now >= total_deadline {
            break;
        }
        if received_indices.len() >= PUBLISH_COUNT as usize {
            break;
        }

        tokio::select! {
            _ = interval.tick(),
                if now < publish_deadline && published_ok + (publish_errors.len() as u32) < PUBLISH_COUNT =>
            {
                let seq = published_ok + (publish_errors.len() as u32);
                let payload = serde_json::json!({
                    "peer_id": a_peer_id.to_string(),
                    "x": (seq as f32) * 1.5_f32,
                    "y": -(seq as f32) * 2.0_f32,
                    "z": 0_i32,
                    "f": 0_u8,
                    "_seq": seq,
                }).to_string();
                match client_a.behaviour_mut().gossipsub.publish(
                    positions.clone(),
                    payload.as_bytes(),
                ) {
                    Ok(_) => { published_ok += 1; }
                    Err(e) => {
                        // Capture the exact error variant so the
                        // assertion failure tells us which mode it is.
                        // `NoPeersSubscribedToTopic` is the production
                        // symptom — the assertion below filters for it
                        // specifically.
                        let label = match &e {
                            PublishError::NoPeersSubscribedToTopic => "NoPeersSubscribedToTopic",
                            PublishError::Duplicate => "Duplicate",
                            PublishError::SigningError(_) => "SigningError",
                            PublishError::MessageTooLarge => "MessageTooLarge",
                            PublishError::TransformFailed(_) => "TransformFailed",
                            PublishError::AllQueuesFull(_) => "AllQueuesFull",
                        };
                        publish_errors.push((seq, label.to_string()));
                    }
                }
            }
            ev = client_a.select_next_some() => { let _ = ev; }
            ev = client_b.select_next_some() => {
                if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message { message, .. }
                )) = ev {
                    if message.topic.as_str() == POSITIONS_TOPIC {
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&message.data) {
                            if let Some(seq) = v.get("_seq").and_then(|s| s.as_u64()) {
                                received_indices.insert(seq as u32);
                            }
                        }
                    }
                }
            }
        }
    }

    let received = received_indices.len() as u32;
    let total_attempted = published_ok + publish_errors.len() as u32;
    eprintln!(
        "positions stress: attempted={total_attempted} ok_sync={published_ok} \
         publish_errors={publish_errors:?} received_distinct={received}",
    );

    // === 8. Assertions — the falsifiable invariants. ===
    // No publish should return NoPeersSubscribedToTopic — this is the
    // exact production symptom the user observed. Catching this is the
    // primary purpose of the test.
    let no_peers_errors: Vec<&(u32, String)> = publish_errors
        .iter()
        .filter(|(_, kind)| kind == "NoPeersSubscribedToTopic")
        .collect();
    assert!(
        no_peers_errors.is_empty(),
        "publish returned NoPeersSubscribedToTopic for {} of {PUBLISH_COUNT} attempts: {no_peers_errors:?}",
        no_peers_errors.len(),
    );

    // Receive rate ≥ 90%. Loose enough to tolerate normal gossipsub
    // jitter under sustained load; tight enough to catch the
    // "messages are silently dropped" failure mode.
    assert!(
        received as f32 / PUBLISH_COUNT as f32 >= 0.9,
        "positions propagation incomplete: published_ok={published_ok} received={received}/{PUBLISH_COUNT} ({:.1}%); errors={publish_errors:?}",
        100.0 * received as f32 / PUBLISH_COUNT as f32,
    );
}
