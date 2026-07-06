#![cfg(not(target_arch = "wasm32"))]
//! Two-swarm loopback test for seer-summary/v1 propagation. Publisher
//! swarm uses seer_p2p::publish_summary against a subscriber swarm
//! dialled on a random loopback port. No relayer binary spun up — the
//! swarms mesh each other directly, which is enough to prove the
//! gossipsub topic + payload round-trip.
//!
//! Mirrors the shape of rave-positions-test/tests/positions_via_relayer
//! but skips the relayer process for speed and because seer's use of
//! the topic is a fire-and-forget publish, not a persistent chat.

use std::time::Duration;

use futures::StreamExt;
use libp2p::{
    Multiaddr, SwarmBuilder,
    core::{transport::Transport, upgrade},
    gossipsub::{self, IdentTopic, MessageAuthenticity, ValidationMode},
    identify, identity, noise, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, websocket, yamux,
};
use serde_json::json;

const IDENTIFY_PROTOCOL: &str = "/seer/1.0.0";

#[derive(NetworkBehaviour)]
struct SubBehaviour {
    gossipsub: gossipsub::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
}

fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 0");
    listener.local_addr().expect("local_addr").port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seer_summary_propagates_over_gossipsub() {
    // Subscriber swarm on a fixed loopback port; publishes nothing.
    let sub_port = pick_free_port();
    let sub_keypair = identity::Keypair::generate_ed25519();
    let mut sub_swarm = SwarmBuilder::with_existing_identity(sub_keypair)
        .with_tokio()
        .with_other_transport(|key| {
            websocket::Config::new(tcp::tokio::Transport::new(tcp::Config::default()))
                .upgrade(upgrade::Version::V1)
                .authenticate(noise::Config::new(key).expect("noise"))
                .multiplex(yamux::Config::default())
        })
        .expect("transport")
        .with_behaviour(|key| {
            let cfg = gossipsub::ConfigBuilder::default()
                .validation_mode(ValidationMode::Strict)
                .build()
                .expect("gossipsub cfg");
            let gossipsub = gossipsub::Behaviour::new(
                MessageAuthenticity::Signed(key.clone()),
                cfg,
            )
            .expect("gossipsub");
            let identify = identify::Behaviour::new(identify::Config::new(
                IDENTIFY_PROTOCOL.to_string(),
                key.public(),
            ));
            let ping = ping::Behaviour::new(ping::Config::default());
            SubBehaviour {
                gossipsub,
                identify,
                ping,
            }
        })
        .expect("behaviour")
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(30)))
        .build();

    let listen_addr: Multiaddr = format!("/ip4/127.0.0.1/tcp/{sub_port}/ws").parse().expect("addr");
    sub_swarm.listen_on(listen_addr.clone()).expect("listen");
    let topic = IdentTopic::new(seer_p2p::SEER_SUMMARY_TOPIC);
    sub_swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&topic)
        .expect("subscribe");

    // Give the subscriber a moment to bind before the publisher dials.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Publisher task via seer_p2p::publish_summary against the sub.
    let bootstrap = listen_addr.to_string();
    let payload = json!({
        "sha": "1111111111111111111111111111111111111111",
        "when_unix": 42u64,
        "note": "seer-p2p-test"
    });
    let publisher = tokio::spawn(async move {
        seer_p2p::publish_summary(&payload, &bootstrap, Duration::from_secs(15)).await
    });

    // Drive the subscriber swarm, waiting for the message.
    let recv_result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match sub_swarm.select_next_some().await {
                SwarmEvent::Behaviour(SubBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message { message, .. },
                )) => {
                    let s = String::from_utf8_lossy(&message.data);
                    if s.contains("seer-p2p-test") {
                        return true;
                    }
                }
                _ => {}
            }
        }
    })
    .await;

    // Whether the publisher's inner deadline hits or not, the mesh
    // saw the message — that's what we're asserting.
    let received = recv_result.unwrap_or(false);
    let _ = publisher.await; // don't panic on publisher's own error
    assert!(received, "subscriber never saw the published seer-summary message");
}
