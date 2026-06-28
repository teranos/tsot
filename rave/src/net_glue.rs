//! Bevy ↔ libp2p glue. The async boot task, the Bevy systems that
//! pull events out of [`crate::net::Net`] and push positions into it,
//! and the `RemotePlayers` map that holds every other peer's
//! last-known position.
//!
//! All wasm32-only — `crate::net::Net` is gated to wasm32 too.

#![cfg(target_arch = "wasm32")]

use bevy::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

use crate::chat;
use crate::error;
use crate::identity;
use crate::net;
use crate::observability::{ErrorLog, Severity};
use crate::room;

/// Production relay multiaddr — shared with roam, served by the relayer
/// binary at `relay.sbvh.nl`. The 12D3KooW… is the relay's deterministic
/// PeerId derived from its persistent identity secret in AWS Secrets
/// Manager; same value roam's JS bridge uses.
pub const RELAY_MULTIADDR: &str =
    "/dns4/relay.sbvh.nl/tcp/443/wss/p2p/12D3KooWMSVxS7ntMVuvVADgZWMZwsjyYmcZvhnyQAJ53PtSJHpN";

/// gossipsub topic. Read by [`crate::drawer::update_net_stats`] for the
/// drawer net-stats line and by every system that publishes or routes
/// a position.
pub const POSITIONS_TOPIC: &str = "rave-positions/v1";

thread_local! {
    /// Bridge between the async boot task (spawn_local on the JS
    /// microtask loop) and Bevy's Update schedule. The boot writes
    /// `Net` here once identity is resolved + Swarm is constructed;
    /// [`install_pending_net`] takes it the next frame and inserts
    /// as a NonSend resource. wasm32 is single-threaded so the
    /// RefCell never races.
    static PENDING_NET: RefCell<Option<net::Net>> = const { RefCell::new(None) };
}

/// R10 — one entry per other peer's last-known position. The Bevy
/// Entity is spawned lazily by [`render_remote_players`] when we first
/// receive a position; subsequent updates only mutate the Transform.
#[derive(Default)]
pub struct RemoteEntry {
    pub pos: Vec3,
    pub last_seen_ms: u64,
    pub entity: Option<Entity>,
}

#[derive(Resource, Default)]
pub struct RemotePlayers(pub HashMap<String, RemoteEntry>);

#[derive(Component)]
pub struct RemotePlayerCell;

/// Awaits the JS identity bridge (load from IndexedDB or mint+persist on
/// first visit), constructs the libp2p Swarm, subscribes to
/// `rave-positions/v1`, and stashes the resulting `Net` for
/// [`install_pending_net`] to pick up.
pub async fn boot_net() {
    use wasm_bindgen::JsCast;

    let load_promise = identity::js_rave_load_identity();
    let identity_bytes: Vec<u8> = match wasm_bindgen_futures::JsFuture::from(load_promise).await {
        Ok(val) if !val.is_null() && !val.is_undefined() => {
            match val.dyn_into::<js_sys::Uint8Array>() {
                Ok(arr) => {
                    let mut bytes = vec![0u8; arr.length() as usize];
                    arr.copy_to(&mut bytes);
                    bytes
                }
                Err(_) => {
                    error::emit_region(
                        error::Severity::Error,
                        "identity-load",
                        "non-Uint8Array from JS bridge",
                        "expected Uint8Array (or null), got something else",
                    );
                    return;
                }
            }
        }
        Ok(_) => {
            // null/undefined — first visit. Generate + persist.
            match identity::generate_identity_protobuf() {
                Ok(fresh) => {
                    let arr = js_sys::Uint8Array::from(fresh.as_slice());
                    let save_promise = identity::js_rave_save_identity(arr);
                    if let Err(e) = wasm_bindgen_futures::JsFuture::from(save_promise).await {
                        error::emit_region(
                            error::Severity::Warn,
                            "identity-save",
                            "IndexedDB save rejected",
                            format!("{e:?}"),
                        );
                    }
                    fresh
                }
                Err(e) => {
                    error::emit_region(
                        error::Severity::Error,
                        "identity-generate",
                        "Ed25519 keypair generation failed",
                        format!("{e:?}"),
                    );
                    return;
                }
            }
        }
        Err(e) => {
            error::emit_region(
                error::Severity::Error,
                "identity-load",
                "IndexedDB load rejected",
                format!("{e:?}"),
            );
            return;
        }
    };

    let net = match net::Net::new(vec![RELAY_MULTIADDR.to_string()], Some(&identity_bytes)) {
        Ok(n) => n,
        Err(e) => {
            error::emit_region(
                error::Severity::Error,
                "net-new",
                "Swarm construction failed",
                format!("{e:?}"),
            );
            return;
        }
    };

    if let Err(e) = net.subscribe(&net::Topic(POSITIONS_TOPIC.to_string())) {
        error::emit_region(
            error::Severity::Error,
            "net-subscribe",
            "subscribe to rave-positions/v1 failed",
            format!("{e:?}"),
        );
        return;
    }

    if let Err(e) = net.subscribe(&net::Topic(chat::CHAT_TOPIC.to_string())) {
        error::emit_region(
            error::Severity::Error,
            "net-subscribe",
            "subscribe to rave-chat/v1 failed",
            format!("{e:?}"),
        );
        return;
    }

    PENDING_NET.with(|cell| *cell.borrow_mut() = Some(net));
}

pub fn install_pending_net(mut maybe_net: NonSendMut<Option<net::Net>>) {
    if maybe_net.is_some() {
        return;
    }
    PENDING_NET.with(|cell| {
        if let Some(n) = cell.borrow_mut().take() {
            *maybe_net = Some(n);
        }
    });
}

/// 10Hz publish of self position to `rave-positions/v1`. Accumulates
/// `delta_secs` into a Local until 100ms have passed, then publishes
/// one RavePosition. Publish failures surface as NetEvent::Error via
/// the drain system; no manual error handling needed here.
pub fn publish_self_position(
    time: Res<Time>,
    mut acc: Local<f32>,
    players: Query<&Transform, With<room::PlayerCell>>,
    maybe_net: NonSend<Option<net::Net>>,
) {
    let Some(n) = maybe_net.as_ref() else {
        return;
    };
    *acc += time.delta_secs();
    if *acc < 0.1 {
        return;
    }
    *acc = 0.0;

    let Some(tf) = players.iter().next() else {
        return;
    };
    let pos = net::RavePosition {
        peer: n.identity().0.clone(),
        x: tf.translation.x,
        y: tf.translation.y,
        z: tf.translation.z,
        at_ms: js_sys::Date::now() as u64,
    };
    let bytes = match serde_json::to_vec(&pos) {
        Ok(b) => b,
        Err(e) => {
            error::emit_region(
                error::Severity::Error,
                "publish-serialize",
                "RavePosition serialize failed",
                format!("{e}"),
            );
            return;
        }
    };
    if let Err(e) = n.publish(&net::Topic(POSITIONS_TOPIC.to_string()), &bytes) {
        error::emit_region(
            error::Severity::Error,
            "publish-send",
            "publish to rave-positions/v1 failed",
            format!("{e:?}"),
        );
    }
}

pub fn drain_net_events(
    maybe_net: NonSend<Option<net::Net>>,
    mut error_log: ResMut<ErrorLog>,
    mut remotes: ResMut<RemotePlayers>,
) {
    let Some(n) = maybe_net.as_ref() else {
        return;
    };
    let self_peer = n.identity().0.clone();
    let now_ms = js_sys::Date::now() as u64;

    for ev in n.poll_events() {
        match ev {
            net::NetEvent::PeerUp { peer, .. } => {
                error_log.emit(Severity::Note, format!("[net] peer up: {}", peer.0));
            }
            net::NetEvent::PeerDown { peer, reason } => {
                error_log.emit(
                    Severity::Warn,
                    format!("[net] peer down: {} ({reason})", peer.0),
                );
            }
            net::NetEvent::Message { topic, bytes, .. } => {
                // Route rave-positions traffic into RemotePlayers (R10).
                // Don't push every gossip message to the drawer — at
                // 10Hz per peer it floods the text node and tanks FPS.
                if topic.0 == POSITIONS_TOPIC {
                    match serde_json::from_slice::<net::RavePosition>(&bytes) {
                        Ok(pos) => {
                            if pos.peer != self_peer {
                                let entry =
                                    remotes.0.entry(pos.peer.clone()).or_default();
                                entry.pos = Vec3::new(pos.x, pos.y, pos.z);
                                entry.last_seen_ms = now_ms;
                            }
                        }
                        Err(e) => {
                            error::emit_region(
                                error::Severity::Error,
                                "decode-rave-position",
                                "malformed RavePosition wire payload",
                                format!("{e}"),
                            );
                        }
                    }
                } else if topic.0 == chat::CHAT_TOPIC {
                    // Hand off to the overlay via __raveChatRecv.
                    // chat::handle_incoming decodes + filters out
                    // self-echoes (gossipsub shouldn't echo, but a
                    // belt-and-braces filter is cheap).
                    chat::handle_incoming(&bytes, &self_peer);
                }
                // Other topics: silently ignored. Drawer stays quiet.
            }
            net::NetEvent::SubscriptionChange {
                topic,
                peer,
                joined,
            } => {
                error_log.emit(
                    Severity::Note,
                    format!(
                        "[net] {} on {} by {}",
                        if joined { "+sub" } else { "-sub" },
                        topic.0,
                        peer.0
                    ),
                );
            }
            net::NetEvent::Error(err) => {
                error_log.emit(Severity::Error, format!("[net] {err:?}"));
            }
        }
    }
}

/// R10 render. Each remote peer becomes a Sphere at the position they
/// last broadcast on `rave-positions/v1`. Y comes from the wire so a
/// peer that drifts off the floor still renders where they say they
/// are. Entries older than 30s get culled with their Bevy entity.
pub fn render_remote_players(
    mut commands: Commands,
    mut remotes: ResMut<RemotePlayers>,
    mut transforms: Query<&mut Transform, With<RemotePlayerCell>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let now_ms = js_sys::Date::now() as u64;
    let stale_cutoff = now_ms.saturating_sub(30_000);

    let stale_peers: Vec<String> = remotes
        .0
        .iter()
        .filter(|(_, e)| e.last_seen_ms < stale_cutoff)
        .map(|(p, _)| p.clone())
        .collect();
    for peer in stale_peers {
        if let Some(entry) = remotes.0.remove(&peer) {
            if let Some(entity) = entry.entity {
                commands.entity(entity).despawn();
            }
        }
    }

    for (_peer, entry) in remotes.0.iter_mut() {
        match entry.entity {
            None => {
                let mesh = meshes.add(Sphere::new(20.0));
                let mat = materials
                    .add(StandardMaterial::from(Color::srgb(0.9, 0.3, 0.85)));
                let id = commands
                    .spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(mat),
                        Transform::from_translation(entry.pos),
                        RemotePlayerCell,
                    ))
                    .id();
                entry.entity = Some(id);
            }
            Some(entity) => {
                if let Ok(mut tf) = transforms.get_mut(entity) {
                    tf.translation = entry.pos;
                }
            }
        }
    }
}
