// Ported from rave/src/trees.rs. Wang-hash-placed forest of TreeTrunk
// + TreeFoliage entities across a 6000×6000 world floor, excluding a
// central clearing + a south-going trail corridor.
//
// Adapted to seer's ECS: no Mesh3d / StandardMaterial (that pipeline
// isn't in seer yet), no Bevy Transform (we use Position from the
// physics port). The algorithm — Wang-hash gating + jitter + AABB
// collider on trunks — is verbatim. The observation is: how many
// entities get spawned, do they leak, how much heap does the ECS
// storage grow after this system runs.
//
// Deterministic: same salt + same coordinates → same forest every
// time. Verifying observability across commits requires this.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::hash::{jitter, wang_hash};
use crate::obs;
use crate::physics::{AabbCollider, Position};

// Ported constants — verbatim values from rave.
const FLOOR_HALF: f32 = 3000.0;
const CLEARING_HALF: f32 = 500.0;
const CELL: f32 = 80.0;
const SPAWN_THRESHOLD: u32 = u32::MAX / 8;
const CLEARING_EXCLUSION: f32 = CLEARING_HALF + 60.0;
const TRAIL_CORRIDOR_HALF: f32 = 70.0;
const TREE_DENSITY_SALT: u32 = 0xC0DE_F00D;

#[derive(Component)]
pub struct TreeTrunk;

#[derive(Component)]
pub struct TreeFoliage;

pub fn setup_trees(mut commands: Commands) {
    let cells_per_side = (FLOOR_HALF * 2.0 / CELL) as i32;
    let origin = -FLOOR_HALF;
    let mut trunks = 0u32;
    let mut foliage = 0u32;

    for ix in 0..cells_per_side {
        for iz in 0..cells_per_side {
            let cell_x = origin + ix as f32 * CELL + CELL / 2.0;
            let cell_z = origin + iz as f32 * CELL + CELL / 2.0;
            if cell_x.hypot(cell_z) < CLEARING_EXCLUSION {
                continue;
            }
            if cell_z > CLEARING_HALF && cell_x.abs() < TRAIL_CORRIDOR_HALF {
                continue;
            }
            if wang_hash(ix, iz, TREE_DENSITY_SALT) >= SPAWN_THRESHOLD {
                continue;
            }
            let jitter_x = jitter(ix, iz, 1) * (CELL * 0.35);
            let jitter_z = jitter(ix, iz, 2) * (CELL * 0.35);
            let tx = cell_x + jitter_x;
            let tz = cell_z + jitter_z;

            commands.spawn((
                TreeTrunk,
                Position(Vec3::new(tx, 30.0, tz)),
                AabbCollider::cuboid(Vec3::new(12.0, 60.0, 12.0)),
            ));
            commands.spawn((TreeFoliage, Position(Vec3::new(tx, 80.0, tz))));
            trunks += 1;
            foliage += 1;
        }
    }
    obs::emit(&format!(
        "[seer.trees] spawned {trunks} trunks + {foliage} foliage entities (deterministic Wang-hash placement)"
    ));
}
