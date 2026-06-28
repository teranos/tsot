#![cfg(not(target_arch = "wasm32"))]
//! End-to-end test for `rave-chat/v1` propagation. Same shape as
//! `positions_via_relayer.rs` — two native client swarms meshed through
//! the actual relayer binary on loopback — but for the chat topic.
//!
//! When this passes, two browsers exchanging chat through
//! `relay.sbvh.nl` is provably correct. Replaces "open two browsers and
//! type".

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

const CHAT_TOPIC: &str = "rave-chat/v1";
const IDENTIFY_PROTOCOL: &str = "/rave/1.0.0";

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
async fn rave_chat_propagates_via_real_relayer() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .expect("crates/ parent")
        .parent()
        .expect("tsot-roam root");
    let relayers_dir = repo_root.join("roam").join("relayers");
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

    let mut client_a = build_client_swarm();
    let mut client_b = build_client_swarm();
    let a_peer_id = *client_a.local_peer_id();

    let chat = IdentTopic::new(CHAT_TOPIC);
    client_a
        .behaviour_mut()
        .gossipsub
        .subscribe(&chat)
        .expect("A subscribe");
    client_b
        .behaviour_mut()
        .gossipsub
        .subscribe(&chat)
        .expect("B subscribe");

    let relay_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/{libp2p_port}/ws")
        .parse()
        .expect("relay multiaddr");
    client_a.dial(relay_addr.clone()).expect("A dial");
    client_b.dial(relay_addr.clone()).expect("B dial");

    // Wait for both clients to see the relayer's subscription on
    // CHAT_TOPIC. Asserts the relayer subscribes to rave-chat/v1 —
    // without that, mesh is empty for this topic and publish silently
    // PublishFailed-s.
    let mut a_saw_relay_subscribed = false;
    let mut b_saw_relay_subscribed = false;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                ev = client_a.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Subscribed { topic, .. }
                    )) = ev {
                        if topic.as_str() == CHAT_TOPIC {
                            a_saw_relay_subscribed = true;
                        }
                    }
                }
                ev = client_b.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Subscribed { topic, .. }
                    )) = ev {
                        if topic.as_str() == CHAT_TOPIC {
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
    .expect("mesh did not form within 30s — relayer never propagated subscribe for rave-chat/v1");

    // A publishes a chat line.
    let payload = serde_json::json!({
        "peer": a_peer_id.to_string(),
        "body": "hello from the dancefloor",
        "at_ms": 1_700_000_000_000_u64,
    })
    .to_string();
    client_a
        .behaviour_mut()
        .gossipsub
        .publish(chat.clone(), payload.as_bytes())
        .expect("A publish chat");

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
                        if message.topic.as_str() == CHAT_TOPIC {
                            return message;
                        }
                    }
                }
            }
        }
    })
    .await
    .expect("B never received A's chat over the real wire");

    assert_eq!(
        received.topic.as_str(),
        CHAT_TOPIC,
        "topic on received message must be rave-chat/v1",
    );
    assert_eq!(
        received.source,
        Some(a_peer_id),
        "signed source must be client A's PeerId (gossipsub Strict-validated)",
    );
    let decoded: serde_json::Value =
        serde_json::from_slice(&received.data).expect("payload is JSON");
    assert_eq!(decoded["peer"], a_peer_id.to_string(), "peer round-trips");
    assert_eq!(
        decoded["body"], "hello from the dancefloor",
        "body round-trips"
    );
    assert_eq!(
        decoded["at_ms"], 1_700_000_000_000_u64,
        "at_ms round-trips"
    );
}
