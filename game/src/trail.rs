// Trail — dark strip on the ground from just north of the dancefloor
// to just short of the player spawn. Ported from rave/src/trail.rs;
// rave used a `Plane3d` mesh + StandardMaterial (bevy_pbr), game
// renders it as one thin flat instanced cube in scene::snapshot_to_
// instances.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::physics::Position;
use crate::room;

#[derive(Component)]
pub struct TrailMarker;

/// Trail runs south from the dancefloor edge toward SPAWN_POS. Width
/// stays narrow so it reads as a path, not a road.
pub const TRAIL_START_Z: f32 = 40.0;
pub const TRAIL_END_Z: f32 = room::SPAWN_POS.z + 40.0;
pub const TRAIL_WIDTH: f32 = 80.0;

pub fn setup_trail(mut commands: Commands) {
    let centre_z = (TRAIL_START_Z + TRAIL_END_Z) * 0.5;
    commands.spawn((TrailMarker, Position(Vec3::new(0.0, 0.5, centre_z))));
}
