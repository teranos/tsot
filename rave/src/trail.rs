//! Trail — a strip of lighter-coloured ground running south from the
//! clearing edge to the player's spawn point. Single thin rectangular
//! `Plane3d` so the surface reads as a marked path on the forest
//! floor without spending dozens of meshes on it.
//!
//! Sits at y=0.1 so it doesn't z-fight the floor at y=0.0. The trees
//! module (`trees::TRAIL_CORRIDOR_HALF`) keeps the woodland off it.

use bevy::prelude::*;

use crate::floorplan::CLEARING_HALF;
use crate::room::SPAWN_POS;

pub fn setup_trail(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // South-going trail: from the clearing's south edge to just shy
    // of the spawn point. Centred on x=0. Width is narrow so it reads
    // as a path, not a road.
    let trail_start_z = CLEARING_HALF;
    let trail_end_z = SPAWN_POS.z + 40.0;
    let trail_centre_z = (trail_start_z + trail_end_z) / 2.0;
    let trail_length = trail_end_z - trail_start_z;
    let trail_width = 80.0;

    let trail_mesh = meshes.add(
        Plane3d::new(Vec3::Y, Vec2::new(trail_width / 2.0, trail_length / 2.0))
            .mesh()
            .build(),
    );
    let trail_mat =
        materials.add(StandardMaterial::from(Color::srgb(0.18, 0.15, 0.10)));
    commands.spawn((
        Mesh3d(trail_mesh),
        MeshMaterial3d(trail_mat),
        Transform::from_xyz(0.0, 0.1, trail_centre_z),
    ));
}
