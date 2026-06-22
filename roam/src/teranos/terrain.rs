//! Heightmap, terracing, water, caves.
//!
//! `surface_z(x, y)` gives the top solid tile (or sea floor when
//! submerged). `tile_at(x, y, z)` resolves any voxel.

use super::hash::{canonical_x, HashDimension};
use super::noise::{fractal_2d, fractal_3d};
use super::{
    TileKind, BREAK_NOISE_FREQUENCY, BREAK_NOISE_OCTAVES, CAVE_NOISE_FREQUENCY,
    CAVE_NOISE_OCTAVES, CAVE_OPEN_THRESHOLD, GRADIENT_THRESHOLD, SURFACE_NOISE_FREQUENCY,
    SURFACE_NOISE_OCTAVES, SURFACE_Z_MAX, SURFACE_Z_MIN, TERRACE_STEP, WORLD_CIRC_X, WORLD_SEED,
    WORLD_Y_LAT, Z_MAX, Z_MIN,
};

// Raw smooth elevation at (x, y). Clamps y into the playable band so
// the gradient computation near the polar boundary doesn't see the
// artificial Z_MIN jump.
fn raw_elevation(x: i32, y: i32) -> f32 {
    let cy = y.clamp(-WORLD_Y_LAT, WORLD_Y_LAT);
    let cx = canonical_x(x);
    let x_period_base = (WORLD_CIRC_X as f32 * SURFACE_NOISE_FREQUENCY) as i32;
    let n = fractal_2d(
        WORLD_SEED,
        cx as f32 * SURFACE_NOISE_FREQUENCY,
        cy as f32 * SURFACE_NOISE_FREQUENCY,
        SURFACE_NOISE_OCTAVES,
        x_period_base,
    );
    let amplitude = (SURFACE_Z_MAX - SURFACE_Z_MIN) as f32 * 0.5;
    let mid = (SURFACE_Z_MAX + SURFACE_Z_MIN) as f32 * 0.5;
    n * amplitude + mid
}

pub fn surface_z(x: i32, y: i32) -> i32 {
    if y.abs() > WORLD_Y_LAT {
        return Z_MIN;
    }

    let raw = raw_elevation(x, y);
    let raw_e = raw_elevation(x + 1, y);
    let raw_s = raw_elevation(x, y + 1);
    let gradient = (raw_e - raw).abs() + (raw_s - raw).abs();

    // Break noise: high-frequency mask that interrupts long ridges into
    // short cliff segments. Period 24 tiles → contiguous "cliff allowed"
    // patches of ~12 tiles separated by ~12-tile smooth gaps. The hash
    // dimension carries the named seed; no hand-typed magic salt.
    let bs_period = (WORLD_CIRC_X as f32 * BREAK_NOISE_FREQUENCY) as i32;
    let cx = canonical_x(x);
    let break_noise = fractal_2d(
        WORLD_SEED ^ HashDimension::TerrainBreakNoise.salt(),
        cx as f32 * BREAK_NOISE_FREQUENCY,
        y as f32 * BREAK_NOISE_FREQUENCY,
        BREAK_NOISE_OCTAVES,
        bs_period,
    );
    let break_active = break_noise > 0.0;

    let z = if gradient > GRADIENT_THRESHOLD && break_active {
        (raw / TERRACE_STEP as f32).round() as i32 * TERRACE_STEP
    } else {
        raw.round() as i32
    };
    z.clamp(SURFACE_Z_MIN, SURFACE_Z_MAX)
}

pub fn tile_at(x: i32, y: i32, z: i32) -> TileKind {
    if !(Z_MIN..=Z_MAX).contains(&z) {
        return TileKind::Air;
    }
    let sz = surface_z(x, y);

    // Below sea level: column has water from sz+1 up to 0.
    // Water-surface tile (z=0) is Shallow if depth ≤ 1, else Deep.
    if sz < 0 {
        let water_depth = -sz;
        if z == 0 {
            return if water_depth <= 1 {
                TileKind::ShallowWater
            } else {
                TileKind::DeepWater
            };
        }
        if z > 0 {
            return TileKind::Air;
        }
        if z > sz {
            return TileKind::DeepWater; // underwater body
        }
        // z <= sz: lake bed and below. Caves apply.
    } else {
        // Land column.
        if z > sz {
            return TileKind::Air;
        }
        if z == sz {
            return TileKind::Grass;
        }
        // z < sz: subsurface. Caves apply.
    }

    // Subsurface: cave-carving 3D noise. Threshold tuned for ~30% open space.
    let cseed = WORLD_SEED ^ HashDimension::TerrainCaveCarve.salt();
    let cx = canonical_x(x);
    let cave_x_period = (WORLD_CIRC_X as f32 * CAVE_NOISE_FREQUENCY) as i32;
    let n = fractal_3d(
        cseed,
        cx as f32 * CAVE_NOISE_FREQUENCY,
        y as f32 * CAVE_NOISE_FREQUENCY,
        z as f32 * CAVE_NOISE_FREQUENCY,
        CAVE_NOISE_OCTAVES,
        cave_x_period,
    );
    if n > CAVE_OPEN_THRESHOLD {
        TileKind::Air
    } else {
        TileKind::Rock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_z_is_deterministic() {
        let a = surface_z(123, -456);
        let b = surface_z(123, -456);
        assert_eq!(a, b);
    }

    #[test]
    fn surface_z_continuous_across_seam() {
        // The seam diff must be no worse than the rest of the world:
        // with terracing, adjacent tiles step by 0 or TERRACE_STEP.
        for y in -200..200 {
            let left = surface_z(WORLD_CIRC_X - 1, y);
            let right = surface_z(0, y);
            let diff = (left - right).abs();
            assert!(
                diff <= TERRACE_STEP,
                "seam discontinuity at y={y}: surface_z({},y)={left}, surface_z(0,y)={right}, |Δ|={diff}",
                WORLD_CIRC_X - 1
            );
        }
    }

    #[test]
    fn most_terrain_is_walkable() {
        let mut walkable = 0;
        let mut total = 0;
        for x in -100..100 {
            for y in -100..100 {
                let a = surface_z(x, y);
                let b = surface_z(x + 1, y);
                if (a - b).abs() <= 1 { walkable += 1; }
                total += 1;
            }
        }
        let frac = walkable as f32 / total as f32;
        assert!(frac > 0.85, "only {:.1}% walkable adjacency in 200x200; expected > 85%", frac * 100.0);
    }

    #[test]
    fn cliffs_exist_but_sparse() {
        let mut cliffs = 0;
        let mut total = 0;
        for x in -200..200 {
            for y in -200..200 {
                let a = surface_z(x, y);
                let b = surface_z(x + 1, y);
                if (a - b).abs() > 1 { cliffs += 1; }
                total += 1;
            }
        }
        let frac = cliffs as f32 / total as f32;
        assert!(cliffs > 0, "no cliffs in 400x400 sample — mask never triggers");
        assert!(frac < 0.10, "cliff fraction {:.2}% too high; should be sparse", frac * 100.0);
    }

    #[test]
    fn surface_z_wraps_in_x() {
        let here = surface_z(7, 100);
        let wrapped = surface_z(7 + WORLD_CIRC_X, 100);
        assert_eq!(here, wrapped);
        let wrapped_neg = surface_z(7 - WORLD_CIRC_X, 100);
        assert_eq!(here, wrapped_neg);
    }

    #[test]
    fn surface_z_within_range() {
        for x in -200..200 {
            for y in -200..200 {
                let z = surface_z(x, y);
                if y.abs() > WORLD_Y_LAT {
                    assert_eq!(z, Z_MIN);
                } else {
                    assert!((SURFACE_Z_MIN..=SURFACE_Z_MAX).contains(&z), "surface_z({x},{y}) = {z}");
                }
            }
        }
    }

    #[test]
    fn tile_at_top_of_air_above_surface() {
        for x in -50..50 {
            for y in -50..50 {
                let sz = surface_z(x, y);
                let air_z = sz.max(0) + 1;
                assert_eq!(tile_at(x, y, air_z), TileKind::Air, "expected Air at ({x},{y},{air_z}) sz={sz}");
            }
        }
    }

    #[test]
    fn tile_at_water_at_sea_level_below_sea() {
        let mut found = false;
        'outer: for x in 0..200 {
            for y in 0..200 {
                let sz = surface_z(x, y);
                if sz < 0 {
                    let t = tile_at(x, y, 0);
                    assert!(matches!(t, TileKind::ShallowWater | TileKind::DeepWater),
                        "expected water at ({x},{y},0) sz={sz}, got {t:?}");
                    found = true;
                    break 'outer;
                }
            }
        }
        assert!(found, "no water column found in 200x200 — surface noise tuned wrong");
    }

    #[test]
    fn tile_at_polar_zone_is_ocean() {
        let y = WORLD_Y_LAT + 100;
        let t = tile_at(0, y, 0);
        assert_eq!(t, TileKind::DeepWater);
    }
}
