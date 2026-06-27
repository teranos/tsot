#![cfg(not(target_arch = "wasm32"))]
//! End-to-end test for rave-positions propagation across the real
//! libp2p stack: two native client swarms meshed through the actual
//! `relayers` binary (test-mode: random keypair, no AWS calls), all on
//! loopback. The wire protocol is the same one rave's browser bundle
//! speaks against the deployed relayer.
//!
//! This replaces the manual "open two browsers" verification of R11 —
//! when this test passes in CI, two browsers meshing through
//! `relay.sbvh.nl` is provably correct.

use std::process::Stdio;
use std::time::Duration;

use libp2p::futures::StreamExt;
use libp2p::{
    core::{transport::Transport, upgrade},
    gossipsub::{self, IdentTopic, MessageAuthenticity, ValidationMode},
    identify, identity, noise, ping,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    tcp, websocket, yamux, Multiaddr, SwarmBuilder,
};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

const POSITIONS_TOPIC: &str = "rave-positions/v1";
const IDENTIFY_PROTOCOL: &str = "/rave/1.0.0";

/// Match the production rave Swarm 1:1 (minus connection_limits since
/// the test clients DO want to accept many peers, unlike browsers which
/// can't accept inbound).
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
async fn rave_position_propagates_via_real_relayer() {
    // === 1. Build the relayer binary. rave and relayers are separate
    // crates; build from inside the relayers/ dir directly. ===
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let relayers_dir = manifest_dir
        .parent()
        .expect("rave/ parent")
        .join("roam")
        .join("relayers");
    let status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&relayers_dir)
        .status()
        .await
        .expect("invoke cargo build");
    assert!(status.success(), "cargo build --release in relayers/ failed");

    let libp2p_port = pick_free_port();
    let status_port = pick_free_port();

    let relayer_bin = relayers_dir
        .join("target")
        .join("release")
        .join("relayers");
    assert!(
        relayer_bin.exists(),
        "relayer binary missing at {relayer_bin:?}"
    );

    // === 2. Spawn the relayer in test mode. ===
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

    let stdout = relayer_child.stdout.take().expect("stdout");
    let mut reader = tokio::io::BufReader::new(stdout).lines();
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Ok(Some(line)) = reader.next_line().await {
            if line.contains("libp2p listening on loopback") {
                return;
            }
        }
        panic!("relayer stdout closed before listening — startup failed");
    })
    .await
    .expect("relayer never reported listening");

    // === 3. Two native rave clients (browsers stand-ins). ===
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

    // === 4. Drive both swarms until each sees a Subscribed on
    // rave-positions/v1 (relayer mesh is ready for this topic). ===
    let mut a_saw_relay_subscribed = false;
    let mut b_saw_relay_subscribed = false;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                ev = client_a.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Subscribed { topic, .. }
                    )) = ev {
                        if topic.as_str() == POSITIONS_TOPIC {
                            a_saw_relay_subscribed = true;
                        }
                    }
                }
                ev = client_b.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Subscribed { topic, .. }
                    )) = ev {
                        if topic.as_str() == POSITIONS_TOPIC {
                            b_saw_relay_subscribed = true;
                        }
                    }
                }
            }
            if a_saw_relay_subscribed && b_saw_relay_subscribed {
                return;
            }
        }
    })
    .await
    .expect("mesh did not form within 30s — relayer never propagated subscribe for rave-positions/v1");

    // === 5. A publishes a RavePosition. ===
    let payload = serde_json::json!({
        "peer": a_peer_id.to_string(),
        "x": 12.5_f32,
        "y": 0.0_f32,
        "z": -34.25_f32,
        "at_ms": 1_700_000_000_000_u64,
    })
    .to_string();
    client_a
        .behaviour_mut()
        .gossipsub
        .publish(positions.clone(), payload.as_bytes())
        .expect("A publish position");

    // === 6. Drive both swarms until B receives the message. ===
    let received = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            tokio::select! {
                ev = client_a.select_next_some() => {
                    let _ = ev;
                }
                ev = client_b.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Message { message, .. }
                    )) = ev {
                        if message.topic.as_str() == POSITIONS_TOPIC {
                            return message;
                        }
                    }
                }
            }
        }
    })
    .await
    .expect("B never received A's position over the real wire");

    // === 7. Assert wire shape + signed author. ===
    assert_eq!(
        received.topic.as_str(),
        POSITIONS_TOPIC,
        "topic on received message must be rave-positions/v1",
    );
    assert_eq!(
        received.source,
        Some(a_peer_id),
        "signed source must be client A's PeerId (gossipsub Strict-validated)",
    );
    let decoded: serde_json::Value =
        serde_json::from_slice(&received.data).expect("payload is JSON");
    assert_eq!(decoded["peer"], a_peer_id.to_string(), "peer round-trips");
    assert_eq!(decoded["x"], 12.5, "x coordinate round-trips");
    assert_eq!(decoded["y"], 0.0, "y coordinate round-trips");
    assert_eq!(decoded["z"], -34.25, "z coordinate round-trips");
    assert_eq!(
        decoded["at_ms"], 1_700_000_000_000_u64,
        "at_ms round-trips"
    );

    // === 8. Tear down (kill_on_drop handles the relayer process). ===
}
