use bevy_app::App;
use bevy_math::Vec3;

use crate::campfire;
use crate::jukebox::Jukebox;
use crate::map::Pin;
use crate::physics::{self, AabbCollider, NpcMarker, PlayerMarker, Position};
use crate::remote_players::{RemotePlayers, color_for_peer};
use crate::template::{PropKind, StructureProp};
use crate::trail::TrailMarker;
use crate::trees;

pub struct RemotePeerDot {
    pub pos: Vec3,
    pub color: [f32; 3],
}

/// One structure prop in the snapshot: position, kind, colour override,
/// size override. Named to keep the SceneSnapshot type readable and
/// clippy quiet.
pub type StructureSnap = (Vec3, PropKind, Option<[f32; 3]>, Option<Vec3>);

pub struct SceneSnapshot {
    pub trees: Vec<(Vec3, f32, &'static crate::tree_mesh::TreeSpecies, bool)>,
    pub obstacles: Vec<Vec3>,
    pub fires: Vec<(Vec3, f32)>,
    pub npcs: Vec<Vec3>,
    pub pins: Vec<Vec3>,
    pub trails: Vec<Vec3>,
    pub remote_peers: Vec<RemotePeerDot>,
    pub structures: Vec<StructureSnap>,
    pub jukeboxes: Vec<Vec3>,
    pub player: Vec3,
}

pub fn snapshot_scene(app: &mut App) -> SceneSnapshot {
    let world = app.world_mut();
    let mut tree_q = world.query::<(&Position, &trees::TreeTrunk)>();
    let trees: Vec<(Vec3, f32, &'static crate::tree_mesh::TreeSpecies, bool)> = tree_q
        .iter(world)
        .map(|(p, t)| (p.0, t.height, t.species, t.stump))
        .collect();
    let mut obs_q = world.query_filtered::<&Position, (
        bevy_ecs::prelude::With<AabbCollider>,
        bevy_ecs::prelude::Without<PlayerMarker>,
        bevy_ecs::prelude::Without<trees::TreeTrunk>,
        bevy_ecs::prelude::Without<campfire::Campfire>,
        bevy_ecs::prelude::Without<StructureProp>,
        bevy_ecs::prelude::Without<Jukebox>,
    )>();
    let obstacles: Vec<Vec3> = obs_q.iter(world).map(|p| p.0).collect();
    let mut juke_q = world.query_filtered::<&Position, bevy_ecs::prelude::With<Jukebox>>();
    let jukeboxes: Vec<Vec3> = juke_q.iter(world).map(|p| p.0).collect();
    let mut fire_q = world.query::<(&Position, &campfire::Campfire)>();
    let fires: Vec<(Vec3, f32)> = fire_q
        .iter(world)
        .map(|(p, f)| (p.0, f.intensity))
        .collect();
    let mut player_q = world
        .query_filtered::<&Position, bevy_ecs::prelude::With<physics::PlayerMarker>>();
    let player = player_q
        .iter(world)
        .next()
        .map(|p| p.0)
        .unwrap_or(Vec3::ZERO);
    let mut npc_q = world.query_filtered::<&Position, bevy_ecs::prelude::With<NpcMarker>>();
    let npcs: Vec<Vec3> = npc_q.iter(world).map(|p| p.0).collect();
    let mut pin_q = world.query_filtered::<&Position, bevy_ecs::prelude::With<Pin>>();
    let pins: Vec<Vec3> = pin_q.iter(world).map(|p| p.0).collect();
    let mut trail_q = world.query_filtered::<&Position, bevy_ecs::prelude::With<TrailMarker>>();
    let trails: Vec<Vec3> = trail_q.iter(world).map(|p| p.0).collect();
    let remote_peers: Vec<RemotePeerDot> = world
        .get_resource::<RemotePlayers>()
        .map(|r| {
            r.0.iter()
                .map(|(peer, e)| RemotePeerDot {
                    pos: e.pos,
                    color: color_for_peer(peer),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut struct_q = world.query::<(&Position, &StructureProp)>();
    let structures: Vec<StructureSnap> = struct_q
        .iter(world)
        .map(|(p, s)| (p.0, s.kind, s.color, s.size))
        .collect();
    SceneSnapshot {
        trees,
        obstacles,
        fires,
        npcs,
        pins,
        trails,
        remote_peers,
        structures,
        jukeboxes,
        player,
    }
}
