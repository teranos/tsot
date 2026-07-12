// Wang-hash-placed forest, ported from rave. The forest is a pure
// function of a world-absolute cell grid — `tree_at_cell(ix, iz)` — so
// it extends infinitely and is generated on demand per chunk (see
// `chunk.rs`) rather than spawned all at once. No FLOOR_HALF, no
// entity-per-tree-for-the-whole-world: the streamer holds only the
// cells near the player.
//
// Deterministic: same salt + same cell → same tree everywhere, every
// session, on every peer.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::hash::{jitter, wang_hash};

/// Cell side in world units. Sparser than rave's 80 — the extra
/// spacing suits the open world; drop toward 80 for a denser wood.
pub const CELL: f32 = 120.0;

const CLEARING_HALF: f32 = 500.0;
const SPAWN_THRESHOLD: u32 = u32::MAX / 8;
const CLEARING_EXCLUSION: f32 = CLEARING_HALF + 60.0;
const TRAIL_CORRIDOR_HALF: f32 = 70.0;
const TREE_DENSITY_SALT: u32 = 0xC0DE_F00D;

/// Trunk render/collision height + foliage height, so the streamer and
/// any caller agree on where the two entities sit.
pub const TRUNK_Y: f32 = 30.0;
pub const FOLIAGE_Y: f32 = 80.0;

#[derive(Component)]
pub struct TreeTrunk;

#[derive(Component)]
pub struct TreeFoliage;

/// The tree at global cell `(ix, iz)`, if the hash + exclusions place
/// one. World-absolute grid: cell centre is `(i + 0.5) * CELL`, valid
/// for any (including negative) coordinate. Returns the ground-plane
/// base position (y = 0); the streamer lifts trunk/foliage to their
/// heights. Pure + deterministic.
pub fn tree_at_cell(ix: i32, iz: i32) -> Option<Vec3> {
    let cell_x = (ix as f32 + 0.5) * CELL;
    let cell_z = (iz as f32 + 0.5) * CELL;
    // Keep the central clearing (rave floor) free of trees.
    if cell_x.hypot(cell_z) < CLEARING_EXCLUSION {
        return None;
    }
    // Keep the south-going trail corridor open.
    if cell_z > CLEARING_HALF && cell_x.abs() < TRAIL_CORRIDOR_HALF {
        return None;
    }
    // Density gate.
    if wang_hash(ix, iz, TREE_DENSITY_SALT) >= SPAWN_THRESHOLD {
        return None;
    }
    let jx = jitter(ix, iz, 1) * (CELL * 0.35);
    let jz = jitter(ix, iz, 2) * (CELL * 0.35);
    Some(Vec3::new(cell_x + jx, 0.0, cell_z + jz))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_at_cell_is_deterministic() {
        assert_eq!(tree_at_cell(17, -23), tree_at_cell(17, -23));
    }

    #[test]
    fn clearing_cell_is_empty() {
        // Cell containing the origin — inside the clearing.
        assert!(tree_at_cell(0, 0).is_none());
    }
}
