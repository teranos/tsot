//! The minimal rave world, ported faithfully from the Bevy source.
//!
//! Constants + the Wang-hash forest placement are copied 1:1 from
//! `rave/src/trees.rs` and `rave/src/room.rs` so the POC reproduces
//! the SAME instance count and layout that stresses the GPU in the
//! real client — the comparison is only fair if the load matches.

use glam::{Mat4, Vec3};

use crate::mesh::Instance;

// --- ported constants (see rave/src/room.rs, trees.rs, floorplan.rs) ---

/// Half-size of the walkable XZ floor. 6 km × 6 km.
pub const FLOOR_HALF: f32 = 3000.0;
/// Player spawn — south of the clearing (rave/src/room.rs).
pub const SPAWN_POS: Vec3 = Vec3::new(0.0, 20.0, 2400.0);
/// Clearing half-extent (rave/src/floorplan.rs CLEARING_HALF).
const CLEARING_HALF: f32 = 500.0;

const CELL: f32 = 80.0;
const SPAWN_THRESHOLD: u32 = u32::MAX / 8;
const CLEARING_EXCLUSION: f32 = CLEARING_HALF + 60.0;
const TRAIL_CORRIDOR_HALF: f32 = 70.0;
const TREE_DENSITY_SALT: u32 = 0xC0DE_F00D;

// Colours (srgb literals from the Bevy StandardMaterials).
const FLOOR_COLOR: [f32; 3] = [0.06, 0.08, 0.05];
const TRUNK_COLOR: [f32; 3] = [0.18, 0.12, 0.07];
const FOLIAGE_COLOR: [f32; 3] = [0.06, 0.18, 0.07];
const PLAYER_COLOR: [f32; 3] = [0.35, 0.85, 0.55];

// Primitive sizes (rave/src/trees.rs, room.rs).
const TRUNK_RADIUS: f32 = 6.0;
const TRUNK_HEIGHT: f32 = 60.0;
const FOLIAGE_RADIUS: f32 = 32.0;
const PLAYER_RADIUS: f32 = 20.0;

/// Splitmix64-shaped 32-bit hash — identical to `rave/src/trees.rs`.
/// Same salt → same forest for every peer, every session.
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

fn jitter(ix: i32, iz: i32, axis_salt: u32) -> f32 {
    let h = wang_hash(ix, iz, axis_salt.wrapping_mul(0x1234_5678));
    (h as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// The three instance buffers the renderer draws, one per unit mesh.
pub struct Instances {
    pub floor: Vec<Instance>,
    pub trunks: Vec<Instance>,
    pub spheres: Vec<Instance>, // foliage + player
    pub tree_count: usize,
}

/// Build every instance in the scene from the ported placement rules.
pub fn build() -> Instances {
    // Floor: unit quad scaled to the half-extent, sitting at y=0.
    let floor = vec![Instance::new(
        Mat4::from_scale(Vec3::new(FLOOR_HALF, 1.0, FLOOR_HALF)),
        FLOOR_COLOR,
    )];

    let mut trunks = Vec::new();
    let mut spheres = Vec::new();

    // Player marker (a sphere, like the Bevy PlayerCell).
    spheres.push(Instance::new(
        Mat4::from_scale_rotation_translation(
            Vec3::splat(PLAYER_RADIUS),
            glam::Quat::IDENTITY,
            SPAWN_POS,
        ),
        PLAYER_COLOR,
    ));

    // Forest — the exact Wang-hash sweep from rave/src/trees.rs.
    let cells_per_side = (FLOOR_HALF * 2.0 / CELL) as i32;
    let origin = -FLOOR_HALF;
    let mut tree_count = 0usize;
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
            let tx = cell_x + jitter(ix, iz, 1) * (CELL * 0.35);
            let tz = cell_z + jitter(ix, iz, 2) * (CELL * 0.35);

            // Trunk — cylinder base on the floor (centre at y=30).
            trunks.push(Instance::new(
                Mat4::from_scale_rotation_translation(
                    Vec3::new(TRUNK_RADIUS, TRUNK_HEIGHT, TRUNK_RADIUS),
                    glam::Quat::IDENTITY,
                    Vec3::new(tx, 30.0, tz),
                ),
                TRUNK_COLOR,
            ));
            // Foliage — sphere above the trunk (centre at y=80).
            spheres.push(Instance::new(
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(FOLIAGE_RADIUS),
                    glam::Quat::IDENTITY,
                    Vec3::new(tx, 80.0, tz),
                ),
                FOLIAGE_COLOR,
            ));
            tree_count += 1;
        }
    }

    Instances { floor, trunks, spheres, tree_count }
}

/// Third-person follow camera, ported from `room.rs::camera_follow`
/// (offset 0,300,250 looking at the player) but slowly orbited around
/// the player so a still screenshot reads as 3D and proves the frame
/// loop is live. `t` is elapsed seconds.
pub fn view_proj(t: f32, aspect: f32) -> Mat4 {
    let base = Vec3::new(0.0, 300.0, 250.0);
    let yaw = t * 0.15;
    let rot = glam::Quat::from_rotation_y(yaw);
    let eye = SPAWN_POS + rot * base;
    let proj = Mat4::perspective_rh(60f32.to_radians(), aspect, 1.0, 12_000.0);
    let view = Mat4::look_at_rh(eye, SPAWN_POS, Vec3::Y);
    proj * view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forest_placement_is_deterministic_and_nonempty() {
        let a = build();
        let b = build();
        assert_eq!(a.tree_count, b.tree_count);
        assert!(a.tree_count > 100, "expected a real forest, got {}", a.tree_count);
        // One trunk per tree; foliage + one player marker among spheres.
        assert_eq!(a.trunks.len(), a.tree_count);
        assert_eq!(a.spheres.len(), a.tree_count + 1);
    }

    #[test]
    fn clearing_is_free_of_trees() {
        let s = build();
        for inst in &s.trunks {
            let m = glam::Mat4::from_cols_array_2d(&inst.model);
            let p = m.w_axis.truncate();
            assert!(
                p.x.hypot(p.z) >= CLEARING_HALF,
                "trunk at {:?} intrudes on the clearing",
                p
            );
        }
    }
}
