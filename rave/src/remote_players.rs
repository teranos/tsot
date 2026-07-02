#![cfg(target_arch = "wasm32")]

use bevy::prelude::*;
use bevy_libp2p::{LayeNet, LibP2PMessage, NetError, NetEvent, Topic};
use std::collections::HashMap;

use crate::POSITIONS_TOPIC;
use crate::error;
use crate::health::{self, Health};
use crate::net::RavePosition;
use bevy_observability::{ErrorLog, Severity};
use crate::room;

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

pub fn publish_self_position(
    time: Res<Time>,
    mut acc: Local<f32>,
    players: Query<&Transform, With<room::PlayerCell>>,
    net: Res<LayeNet>,
) {
    *acc += time.delta_secs();
    if *acc < 0.1 {
        return;
    }
    *acc = 0.0;

    let Some(tf) = players.iter().next() else {
        return;
    };
    let pos = RavePosition {
        peer: net.identity().0.clone(),
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
    if let Err(e) = net.publish(&Topic(POSITIONS_TOPIC.to_string()), &bytes) {
        error::emit_region(
            error::Severity::Error,
            "publish-send",
            "publish to rave-positions/v1 failed",
            format!("{e:?}"),
        );
    }
}

pub fn drain_net_events(
    net: Res<LayeNet>,
    mut reader: MessageReader<LibP2PMessage>,
    mut error_log: ResMut<ErrorLog>,
    mut remotes: ResMut<RemotePlayers>,
    mut health: ResMut<Health>,
) {
    let self_peer = net.identity().0.clone();
    let now_ms = js_sys::Date::now() as u64;

    for msg in reader.read() {
        match &msg.0 {
            NetEvent::PeerUp { peer, .. } => {
                error_log.emit(Severity::Note, format!("[net] peer up: {}", peer.0));
            }
            NetEvent::PeerDown { peer, reason } => {
                error_log.emit(
                    Severity::Warn,
                    format!("[net] peer down: {} ({reason})", peer.0),
                );
            }
            NetEvent::Message { topic, bytes, .. } => {
                // Any message on the topic — publish OR subscribe path
                // — proves the mesh works for this topic. Resolves any
                // active PublishFailing health entry keyed to it.
                health::record_message_seen(&mut health, &topic.0);
                if topic.0 == POSITIONS_TOPIC {
                    match serde_json::from_slice::<RavePosition>(bytes) {
                        Ok(pos) => {
                            if pos.peer != self_peer {
                                let entry = remotes.0.entry(pos.peer.clone()).or_default();
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
                }
            }
            NetEvent::SubscriptionChange {
                topic,
                peer,
                joined,
            } => {
                error_log.emit(
                    Severity::Note,
                    format!(
                        "[net] {} on {} by {}",
                        if *joined { "+sub" } else { "-sub" },
                        topic.0,
                        peer.0
                    ),
                );
            }
            // PublishFailed is a durable condition, not an ephemeral
            // event — a solo peer on a 10 Hz publish topic emits ~10
            // of these per second. Route into Health so the panel
            // shows one live row instead of a red waterfall. Other
            // NetError variants stay in the log where they belong
            // (rare, one-shot).
            NetEvent::Error(NetError::PublishFailed { topic, reason }) => {
                health::record_publish_failed(&mut health, &topic.0, reason, now_ms);
            }
            NetEvent::Error(other) => {
                error_log.emit(Severity::Error, format!("[net] {other:?}"));
            }
        }
    }
}

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
        if let Some(entry) = remotes.0.remove(&peer)
            && let Some(entity) = entry.entity
        {
            commands.entity(entity).despawn();
        }
    }

    for (_peer, entry) in remotes.0.iter_mut() {
        match entry.entity {
            None => {
                let mesh = meshes.add(Sphere::new(20.0));
                let mat = materials.add(StandardMaterial::from(Color::srgb(0.9, 0.3, 0.85)));
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
