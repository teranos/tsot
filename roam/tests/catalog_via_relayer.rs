#![cfg(not(target_arch = "wasm32"))]
//! End-to-end test for catalog publish: the relayer announces its
//! card catalog on `CATALOG_TOPIC` over the real libp2p stack, and a
//! native client subscribed to the topic receives the bytes and parses
//! them through `roam::catalog::parse_catalog_json`.
//!
//! Mirrors the M6 harness — one real relayer subprocess, one native
//! libp2p client meshed through it, all on loopback. The whole point
//! is to exercise the exact wire path the deployed relayer + the
//! browser-side worker speak.

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

const CATALOG_TOPIC: &str = "roam-catalog/v1";
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
async fn relayer_publishes_catalog_and_client_receives_it() {
    // === 1. Build relayer binary (release). ===
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let relayers_dir = std::path::PathBuf::from(manifest_dir).join("relayers");
    let status = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&relayers_dir)
        .status()
        .await
        .expect("invoke cargo build");
    assert!(status.success(), "cargo build --release in relayers/ failed");

    // === 2. Pick free ports and locate the binary. ===
    let libp2p_port = pick_free_port();
    let status_port = pick_free_port();
    let relayer_bin = relayers_dir
        .join("target")
        .join("release")
        .join("relayers");
    assert!(relayer_bin.exists(), "relayer binary missing at {relayer_bin:?}");

    // === 3. Spawn relayer in test mode. ===
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

    // === 4. Wait for `libp2p listening on loopback`. ===
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

    // === 5. Build client swarm + subscribe to CATALOG_TOPIC. ===
    let mut client = build_client_swarm();
    let catalog = IdentTopic::new(CATALOG_TOPIC);
    client
        .behaviour_mut()
        .gossipsub
        .subscribe(&catalog)
        .expect("client subscribe");

    // === 6. Dial the relayer. ===
    let relay_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/{libp2p_port}/ws")
        .parse()
        .expect("relay multiaddr");
    client.dial(relay_addr).expect("client dial");

    // === 7. Drive the swarm until a Message arrives on CATALOG_TOPIC. ===
    // The relayer's `catalog_interval` fires immediately at startup —
    // first publish goes out as soon as a subscribed peer appears on
    // the mesh. Subsequent publishes every 30s. We expect the first
    // arrival within a few seconds; the timeout is generous because
    // the mesh-form handshake itself takes a second or two.
    let received = tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let ev = client.select_next_some().await;
            if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                gossipsub::Event::Message { message, .. },
            )) = ev
            {
                if message.topic.as_str() == CATALOG_TOPIC {
                    return message;
                }
            }
        }
    })
    .await
    .expect("client never received a catalog message — relayer publish path is broken");

    // === 8. Verify wire shape directly via serde_json — don't link the
    // roam lib (panic-strategy mismatch with the integration-test crate).
    // The relayer's `catalog_json()` in `relayers/src/main.rs` is the
    // contract this test pins. ===
    let parsed: serde_json::Value =
        serde_json::from_slice(&received.data).expect("catalog payload is valid JSON");
    let arr = parsed.as_array().expect("catalog payload is a JSON array");
    assert!(
        !arr.is_empty(),
        "relayer published an empty catalog — should have at least the hardcoded stubs"
    );
    let ids: Vec<&str> = arr
        .iter()
        .filter_map(|e| e.get("id").and_then(|v| v.as_str()))
        .collect();
    assert!(
        ids.contains(&"battle-captain"),
        "expected 'battle-captain' in published catalog, got: {ids:?}"
    );
    assert!(
        ids.contains(&"anaconda"),
        "expected 'anaconda' in published catalog, got: {ids:?}"
    );
    let battle_captain = arr
        .iter()
        .find(|e| e.get("id").and_then(|v| v.as_str()) == Some("battle-captain"))
        .expect("battle-captain entry exists");
    assert_eq!(
        battle_captain.get("name").and_then(|v| v.as_str()),
        Some("Battle Captain"),
        "name round-trip"
    );
}
