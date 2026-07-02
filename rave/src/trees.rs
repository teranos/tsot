//! Forest — primitive trees Wang-hash placed across the world floor,
//! excluded from the clearing radius (`floorplan::CLEARING_HALF`) and
//! from a narrow corridor along the trail south of the clearing so
//! the player can walk in unobstructed.
//!
//! Each tree is two entities — a tall Cylinder trunk + a Sphere of
//! foliage above it. No textures, no GLBs; same primitive aesthetic
//! the rest of the placeholder geometry uses until real assets land.

use bevy::prelude::*;

use crate::floorplan::CLEARING_HALF;
use crate::physics::AabbCollider;
use crate::room::FLOOR_HALF;

/// Side length of one Wang-hash placement cell (world units). 80m
/// gives forest density around 1 tree per 80×80 = 6400 m² area at
/// the density odds below — sparse enough to walk through, dense
/// enough to look like woodland.
const CELL: f32 = 80.0;

/// Per-cell hash value below this threshold (out of `u32::MAX`)
/// spawns a tree. ~12% density = ~1-in-8 cells.
const SPAWN_THRESHOLD: u32 = u32::MAX / 8;

/// Cells within this radius around the origin are skipped — keeps
/// the clearing free of trees + leaves a buffer around it so the
/// woodland doesn't crowd the dancefloor edge.
const CLEARING_EXCLUSION: f32 = CLEARING_HALF + 60.0;

/// Trail corridor — cells within this half-width along x=0 from the
/// clearing edge southward to the world boundary are skipped so the
/// player can walk in.
const TRAIL_CORRIDOR_HALF: f32 = 70.0;

/// Salt for the tree-density hash. Picking a new salt would reshuffle
/// the whole forest deterministically — same value across sessions
/// and across peers so everyone sees the same trees.
const TREE_DENSITY_SALT: u32 = 0xC0DE_F00D;

pub fn setup_trees(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Shared meshes + materials — one allocation each, every tree
    // re-uses them. With ~hundreds of trees this matters; without
    // sharing we'd churn through Mesh + StandardMaterial allocations
    // at startup.
    let trunk_mesh = meshes.add(Cylinder::new(6.0, 60.0));
    let foliage_mesh = meshes.add(Sphere::new(32.0));
    let trunk_mat =
        materials.add(StandardMaterial::from(Color::srgb(0.18, 0.12, 0.07)));
    let foliage_mat =
        materials.add(StandardMaterial::from(Color::srgb(0.06, 0.18, 0.07)));

    let cells_per_side = (FLOOR_HALF * 2.0 / CELL) as i32;
    let origin = -FLOOR_HALF;
    for ix in 0..cells_per_side {
        for iz in 0..cells_per_side {
            let cell_x = origin + ix as f32 * CELL + CELL / 2.0;
            let cell_z = origin + iz as f32 * CELL + CELL / 2.0;
            // Exclude the clearing.
            if cell_x.hypot(cell_z) < CLEARING_EXCLUSION {
                continue;
            }
            // Exclude the trail corridor (south of clearing edge).
            if cell_z > CLEARING_HALF && cell_x.abs() < TRAIL_CORRIDOR_HALF {
                continue;
            }
            // Wang-hash density check.
            if wang_hash(ix, iz, TREE_DENSITY_SALT) >= SPAWN_THRESHOLD {
                continue;
            }
            // Sub-cell jitter so trees don't read as a regular grid.
            let jitter_x = jitter(ix, iz, 1) * (CELL * 0.35);
            let jitter_z = jitter(ix, iz, 2) * (CELL * 0.35);
            let tx = cell_x + jitter_x;
            let tz = cell_z + jitter_z;
            // Trunk — centre at y=30 puts its base on the floor.
            // AABB matches the trunk cylinder (radius 6, height 60);
            // foliage sphere above (y=48-112) sits outside the
            // player's reach (y=0-40) so no collider on it.
            commands.spawn((
                Mesh3d(trunk_mesh.clone()),
                MeshMaterial3d(trunk_mat.clone()),
                Transform::from_xyz(tx, 30.0, tz),
                AabbCollider::cuboid(Vec3::new(12.0, 60.0, 12.0)),
            ));
            // Foliage — sphere centred at y=80 sits above the trunk.
            commands.spawn((
                Mesh3d(foliage_mesh.clone()),
                MeshMaterial3d(foliage_mat.clone()),
                Transform::from_xyz(tx, 80.0, tz),
            ));
        }
    }
}

/// Splitmix64-shaped 32-bit hash. Mixes the two integer coordinates +
/// a salt into one value used to gate cell occupancy and per-cell
/// jitter. Deterministic — same world for everyone, every session.
fn wang_hash(ix: i32, iz: i32, salt: u32) -> u32 {
    let mut h = (ix as u32)
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add((iz as u32).wrapping_mul(0x85EB_CA77))
        .wrapping_add(salt);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h
}

/// Returns a deterministic jitter value in `[-1.0, 1.0]` for a given
/// cell + axis salt. Lets two trees in the same cell row land at
/// slightly different x/z so the forest doesn't read as a grid.
fn jitter(ix: i32, iz: i32, axis_salt: u32) -> f32 {
    let h = wang_hash(ix, iz, axis_salt.wrapping_mul(0x1234_5678));
    (h as f32 / u32::MAX as f32) * 2.0 - 1.0
}

