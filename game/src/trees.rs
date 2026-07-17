// Wang-hash-placed forest, ported from rave and grown up. The forest
// is a pure function of a world-absolute cell grid — `tree_at_cell` —
// so it extends infinitely and streams per chunk (see `chunk.rs`).
//
// Density is non-uniform: a low-frequency value-noise field thins the
// woods into clumps and clearings rather than an even scatter. Each
// tree also gets a hash-varied height, and renders as a brown trunk
// under a green canopy (see scene.rs).
//
// Deterministic: same salt + same cell → same tree everywhere.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::hash::wang_hash;

/// Cell side in world units.
pub const CELL: f32 = 120.0;

const CLEARING_HALF: f32 = 500.0;
const CLEARING_EXCLUSION: f32 = CLEARING_HALF + 60.0;
const TRAIL_CORRIDOR_HALF: f32 = 70.0;
const TREE_DENSITY_SALT: u32 = 0xC0DE_F00D;
const TREE_HEIGHT_SALT: u32 = 0x7EE7_0009;

/// Peak local tree probability (in the densest patches). Averaged over
/// the noise field the effective density is well under this, so the
/// woods read as clumpy patches with clearings.
const DENSITY_PEAK: f32 = 0.20;
/// Value-noise lattice spacing — the scale of forest patches/clearings.
const NOISE_CELL: f32 = 1600.0;

/// Trees span this height range (world units) — all far taller than the
/// 220-tall buildings, but varied.
const TREE_MIN_H: f32 = 320.0;
const TREE_MAX_H: f32 = 760.0;

/// A tree: its `height` drives the trunk + canopy render and collider;
/// `species` is carried DATA (not a render-time hash) so an authored
/// CDDA tree can be the species its map names, not one guessed from
/// position. Procedural trees fill it from `species_for_pos`.
#[derive(Component)]
pub struct TreeTrunk {
    pub height: f32,
    pub species: &'static crate::tree_mesh::TreeSpecies,
    /// A cut stump — the short remainder of a felled tree of THIS species
    /// (an oak stump keeps oak bark). Rendered as a stout bole with a pale
    /// cut face, no crown. `false` for a whole living tree.
    pub stump: bool,
}

fn hash01(ix: i32, iz: i32, salt: u32) -> f32 {
    wang_hash(ix, iz, salt) as f32 / u32::MAX as f32
}

/// Height for an authored (CDDA) tree at a world position — short and
/// NEARLY uniform (tended, planted trees, not wild old-growth), so an
/// orchard's rows line up instead of a jumble of mismatched crowns. The
/// species' `authored_scale` lifts it off the shared base (an apple
/// reads a touch bigger than a plain sapling) without reintroducing the
/// wild height jumble. Deterministic — peers agree via the pure tile
/// hash.
pub fn authored_height(x: f32, z: f32, species: &crate::tree_mesh::TreeSpecies) -> f32 {
    let ix = (x / CELL).round() as i32;
    let iz = (z / CELL).round() as i32;
    (260.0 + hash01(ix, iz, TREE_HEIGHT_SALT) * 40.0) * species.authored_scale
}

/// A small fraction of procedural forest trees are old cut stumps —
/// deterministic per tile so peers agree. The species is still chosen by
/// `species_for_pos`; this only decides the tree was felled.
pub fn is_stump_at(x: f32, z: f32) -> bool {
    let ix = (x / CELL).round() as i32;
    let iz = (z / CELL).round() as i32;
    hash01(ix, iz, 0x0057_0F09) < 0.06
}

/// Smooth value noise in [0,1] at world (x,z) — bilinear-interpolated
/// hashed lattice with a smoothstep fade. Low frequency → big patches.
fn density_noise(x: f32, z: f32) -> f32 {
    let (gx, gz) = (x / NOISE_CELL, z / NOISE_CELL);
    let (x0, z0) = (gx.floor(), gz.floor());
    let (fx, fz) = (gx - x0, gz - z0);
    let (ix0, iz0) = (x0 as i32, z0 as i32);
    let salt = 0x0F0_5EED;
    let a = hash01(ix0, iz0, salt);
    let b = hash01(ix0 + 1, iz0, salt);
    let c = hash01(ix0, iz0 + 1, salt);
    let d = hash01(ix0 + 1, iz0 + 1, salt);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sz = fz * fz * (3.0 - 2.0 * fz);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sz
}

/// The tree at global cell `(ix, iz)`, if the noise-modulated hash +
/// exclusions place one — with its position (y=0 base) and height.
/// Pure + deterministic.
pub fn tree_at_cell(ix: i32, iz: i32) -> Option<(Vec3, f32)> {
    let cell_x = (ix as f32 + 0.5) * CELL;
    let cell_z = (iz as f32 + 0.5) * CELL;
    if cell_x.hypot(cell_z) < CLEARING_EXCLUSION {
        return None;
    }
    if cell_z > CLEARING_HALF && cell_x.abs() < TRAIL_CORRIDOR_HALF {
        return None;
    }
    // Clumpy density: local probability = peak × noise², so clearings
    // (low noise) open right up.
    let noise = density_noise(cell_x, cell_z);
    let local = DENSITY_PEAK * noise * noise;
    if hash01(ix, iz, TREE_DENSITY_SALT) >= local {
        return None;
    }
    let jx = (hash01(ix, iz, 1) * 2.0 - 1.0) * (CELL * 0.35);
    let jz = (hash01(ix, iz, 2) * 2.0 - 1.0) * (CELL * 0.35);
    let height = TREE_MIN_H + hash01(ix, iz, TREE_HEIGHT_SALT) * (TREE_MAX_H - TREE_MIN_H);
    Some((Vec3::new(cell_x + jx, 0.0, cell_z + jz), height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_at_cell_is_deterministic() {
        assert_eq!(tree_at_cell(17, -23), tree_at_cell(17, -23));
    }

    #[test]
    fn authored_apple_stands_a_little_taller_than_a_plain_authored_tree() {
        use crate::tree_mesh::{APPLE, OAK};
        // Authored height is near-uniform, but species scale it: an
        // orchard apple should read bigger than a yard oak/sapling at the
        // SAME tile (so `authored_scale` actually reaches the height).
        for &(x, z) in &[(0.0, 0.0), (240.0, -720.0), (-3600.0, 1200.0)] {
            let apple = authored_height(x, z, &APPLE);
            let oak = authored_height(x, z, &OAK);
            assert!(apple > oak, "apple {apple} should top oak {oak} at ({x},{z})");
        }
    }

    #[test]
    fn clearing_cell_is_empty() {
        assert!(tree_at_cell(0, 0).is_none());
    }

    #[test]
    fn heights_vary_and_stay_taller_than_buildings() {
        let mut heights = Vec::new();
        for ix in -60..60 {
            for iz in -60..60 {
                if let Some((_, h)) = tree_at_cell(ix, iz) {
                    assert!(h > 220.0, "tree {h} shorter than a building");
                    heights.push(h);
                }
            }
        }
        assert!(!heights.is_empty());
        let (min, max) = heights.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &h| {
            (lo.min(h), hi.max(h))
        });
        assert!(max - min > 100.0, "expected varied heights, got {min}..{max}");
    }
}
