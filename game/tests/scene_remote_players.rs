// Wires the RemotePlayers ECS resource into the scene snapshot so the
// browser render loop turns each remote peer into a visible cube.

use bevy_app::App;
use bevy_math::Vec3;

use game::remote_players::{RemoteEntry, RemotePlayers};
use game::scene::{snapshot_scene, snapshot_to_instances};

#[test]
fn remote_peer_lands_in_snapshot() {
    let mut app = App::new();
    let mut remotes = RemotePlayers::default();
    remotes.0.insert(
        "peer1".into(),
        RemoteEntry {
            pos: Vec3::new(100.0, 0.0, 200.0),
            last_seen_ms: 1_000,
        },
    );
    app.insert_resource(remotes);

    let snap = snapshot_scene(&mut app);
    assert_eq!(snap.remote_peers.len(), 1);
    assert_eq!(snap.remote_peers[0].pos, Vec3::new(100.0, 0.0, 200.0));
}

#[test]
fn remote_peer_becomes_scene_instance() {
    let mut app = App::new();
    let mut remotes = RemotePlayers::default();
    remotes.0.insert(
        "peer1".into(),
        RemoteEntry {
            pos: Vec3::new(100.0, 0.0, 200.0),
            last_seen_ms: 1_000,
        },
    );
    app.insert_resource(remotes);

    let snap = snapshot_scene(&mut app);
    let instances = snapshot_to_instances(&snap);
    let hit = instances.iter().find(|inst| {
        (inst.pos[0] - 100.0).abs() < 0.001 && (inst.pos[2] - 200.0).abs() < 0.001
    });
    assert!(hit.is_some(), "no scene instance at remote peer x=100, z=200");
}

#[test]
fn no_remote_peers_leaves_scene_unchanged() {
    let mut app = App::new();
    app.insert_resource(RemotePlayers::default());
    let snap = snapshot_scene(&mut app);
    assert!(snap.remote_peers.is_empty());
}
