// Named world pins the scene hangs from. Ported from rave/src/map.rs,
// pure-data slice: the `Pin` enum + one `pub const Vec3` per zone.
// Rave's render systems (rod/sphere/label overlay via bevy_pbr +
// world_to_viewport UI text) don't cross the boundary — game paints
// pins as yellow marker cubes via scene::snapshot_to_instances.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::physics::Position;

/// Discriminator on every named zone-anchor entity. Attached to a
/// Position-carrying entity spawned at the pin's world coordinate;
/// future zone contents will treat these as parents.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    Stage,
    Dancefloor,
    BarZone,
    Trail,
    Campfire,
}

/// Stage — north edge of the clearing.
pub const STAGE: Vec3 = Vec3::new(0.0, 0.0, -500.0);

/// Dancefloor — clearing origin.
pub const DANCEFLOOR: Vec3 = Vec3::new(0.0, 0.0, 0.0);

/// Bar zone — west side of the clearing.
pub const BAR_ZONE: Vec3 = Vec3::new(-460.0, 0.0, 0.0);

/// Trail — south corridor from the clearing edge toward the player
/// spawn.
pub const TRAIL: Vec3 = Vec3::new(0.0, 0.0, 1470.0);

/// Campfire — just south of the dancefloor edge.
pub const CAMPFIRE: Vec3 = Vec3::new(-800.0, 0.0, 0.0);

/// Spawn one anchor entity per named zone. Marker cubes render via
/// scene::snapshot_to_instances so the developer can eyeball where
/// each zone sits before any zone content is attached.
pub fn setup_pins(mut commands: Commands) {
    for (pin, pos) in [
        (Pin::Stage, STAGE),
        (Pin::Dancefloor, DANCEFLOOR),
        (Pin::BarZone, BAR_ZONE),
        (Pin::Trail, TRAIL),
        (Pin::Campfire, CAMPFIRE),
    ] {
        commands.spawn((pin, Position(pos)));
    }
}
