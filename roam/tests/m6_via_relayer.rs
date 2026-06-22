#![cfg(not(target_arch = "wasm32"))]
//! End-to-end test for M6 flower-pickup propagation across the real
//! libp2p stack: two native client swarms meshed through the actual
//! `relayers` binary (test-mode: random keypair, no AWS calls), all on
//! loopback. The wire protocol — gossipsub subscribe announcements,
//! mesh heartbeat, signed-source verification, fan-out — is the same
//! protocol the browser worker speaks against the deployed relayer.
//!
//! Why this exists instead of a mocked variant:
//!   The 0.3.6 bug that lost a session of dev time was a *missing
//!   subscribe announcement* — the client never told the relayer it
//!   wanted `roam-pickups/v1`. A mock relayer that always-forwards
//!   would have passed every assertion and shipped the bug. The only
//!   test that catches subscribe-propagation failures is one that
//!   runs the real protocol. This is that test.
//!
//! What we trust the libp2p crate for, by NOT testing it here:
//!   - The cryptography (noise handshake, signed message envelopes).
//!   - The mesh heartbeat scheduling.
//!   - The actual byte framing of gossipsub RPCs.
//!
//! These belong to libp2p's own test suite; we test *our* wiring on
//! top of them.
//!
//! Cost: this test spawns a subprocess + builds two native swarms, so
//! it takes a few seconds. The unit-style mock in `src/net/state.rs`
//! is marked `#[ignore]` and superseded by this test.

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

const PICKUPS_TOPIC: &str = "roam-pickups/v1";
const IDENTIFY_PROTOCOL: &str = "/roam/1.0.0";

/// Match the production worker's behaviour set 1:1. Anything the
/// production node has that this test client doesn't have could mask a
/// protocol-version mismatch the wire would otherwise surface.
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

/// Pick a free TCP port on loopback by binding a temporary listener
/// at port 0 and reading the OS-assigned port back. There's a TOCTOU
/// race between drop and the relayer's bind, but in practice this is
/// the standard pattern for test-port allocation and the window is
/// microseconds.
fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 0");
    listener.local_addr().expect("local_addr").port()
}

#[tokio::test(flavor = "multi_thread")]
async fn m6_pickup_propagates_via_real_relayer() {
    // === 1. Build the relayer binary (release). roam and relayers are
    // separate crates (no cargo workspace at the repo level), so
    // `cargo build -p relayers` from `roam/` doesn't see the package.
    // Build from inside `relayers/` instead — the output lands at
    // `relayers/target/release/relayers`. Cheap re-invocation if
    // already built. ===
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
    // `tracing_subscriber::fmt()` writes to STDOUT by default
    // (unlike most loggers that default to stderr) — verified
    // empirically by running the relayer with the test env vars and
    // observing the stream.
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

    // === 5. Build two client swarms. ===
    let mut client_a = build_client_swarm();
    let mut client_b = build_client_swarm();
    let a_peer_id = *client_a.local_peer_id();

    // === 6. Both subscribe to PICKUPS_TOPIC and dial the relayer. ===
    let pickups = IdentTopic::new(PICKUPS_TOPIC);
    client_a
        .behaviour_mut()
        .gossipsub
        .subscribe(&pickups)
        .expect("A subscribe");
    client_b
        .behaviour_mut()
        .gossipsub
        .subscribe(&pickups)
        .expect("B subscribe");

    let relay_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/{libp2p_port}/ws")
        .parse()
        .expect("relay multiaddr");
    client_a.dial(relay_addr.clone()).expect("A dial");
    client_b.dial(relay_addr.clone()).expect("B dial");

    // === 7. Drive both swarms until each has seen a Subscribed event
    // on `roam-pickups/v1` (meaning the relayer announced its own
    // subscription) — that's the production-meaningful "mesh ready"
    // signal for this topic. ===
    let mut a_saw_relay_subscribed = false;
    let mut b_saw_relay_subscribed = false;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                ev = client_a.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Subscribed { topic, .. }
                    )) = ev {
                        if topic.as_str() == PICKUPS_TOPIC {
                            a_saw_relay_subscribed = true;
                        }
                    }
                }
                ev = client_b.select_next_some() => {
                    if let SwarmEvent::Behaviour(ClientBehaviourEvent::Gossipsub(
                        gossipsub::Event::Subscribed { topic, .. }
                    )) = ev {
                        if topic.as_str() == PICKUPS_TOPIC {
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
    .expect("mesh did not form within 30s — relayer never propagated subscribe");

    // === 8. A publishes a pickup wire message. ===
    let payload = serde_json::json!({"x": 42_i32, "y": -7_i32}).to_string();
    client_a
        .behaviour_mut()
        .gossipsub
        .publish(pickups.clone(), payload.as_bytes())
        .expect("A publish_pickup");

    // === 9. Drive both swarms until B sees the Message event. ===
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
                        if message.topic.as_str() == PICKUPS_TOPIC {
                            return message;
                        }
                    }
                }
            }
        }
    })
    .await
    .expect("B never received A's pickup over the real wire");

    // === 10. Assert wire shape + author identity. ===
    assert_eq!(
        received.topic.as_str(),
        PICKUPS_TOPIC,
        "topic on received message must be the pickups topic",
    );
    assert_eq!(
        received.source,
        Some(a_peer_id),
        "signed source must be client A's PeerId (gossipsub Strict-validated)",
    );
    let decoded: serde_json::Value =
        serde_json::from_slice(&received.data).expect("payload is JSON");
    assert_eq!(decoded["x"], 42, "x coordinate round-trips");
    assert_eq!(decoded["y"], -7, "y coordinate round-trips");

    // === 11. Tear down (kill_on_drop handles the relayer process). ===
}
