//! Flower worldgen: types, rarity tables, `flower_at`.
//!
//! Every flower's appearance is derived from its own hash dimension off
//! the same `(x, y)` so peers compute identical results without
//! coordination. Picking up the same flower from two peers' viewpoints
//! yields the same `Flower` value before any state propagates.

use super::hash::{pick_weighted, splitmix64, weight_table_sum, world_hash, HashDimension};
use super::terrain::surface_z;
use super::WORLD_Y_LAT;

/// Petal-edge / petal-center color. Rarity weights are declared in
/// `FLOWER_COLOR_WEIGHTS` below; the const sum-check guards them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FlowerColor {
    Red = 0,
    Yellow = 1,
    Blue = 2,
    Purple = 3,
    Azure = 4,
    Pink = 5,
    Glow = 6,
}

impl FlowerColor {
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            FlowerColor::Red => [0xee, 0x44, 0x44],
            FlowerColor::Yellow => [0xff, 0xdd, 0x44],
            FlowerColor::Blue => [0x44, 0x88, 0xff],
            FlowerColor::Purple => [0xaa, 0x44, 0xff],
            FlowerColor::Azure => [0x44, 0xcc, 0xff],
            FlowerColor::Pink => [0xff, 0x99, 0xcc],
            FlowerColor::Glow => [0xff, 0xff, 0xff],
        }
    }
}

/// Core-center color. Black is super-mega-rare.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FlowerCore {
    White = 0,
    Yellow = 1,
    Black = 2,
}

impl FlowerCore {
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            FlowerCore::White => [0xff, 0xff, 0xff],
            FlowerCore::Yellow => [0xff, 0xdd, 0x44],
            FlowerCore::Black => [0x10, 0x10, 0x10],
        }
    }
}

/// Outer-edge color of the core's radial gradient. White (most common),
/// or matches one of the petal colors.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CoreEdge {
    White = 0,
    MatchPetalCenter = 1,
    MatchPetalEdge = 2,
}

// ----- flower weight tables -----

pub(super) const FLOWER_COLOR_WEIGHTS: [(FlowerColor, u16); 7] = [
    (FlowerColor::Red, 350),
    (FlowerColor::Yellow, 350),
    (FlowerColor::Blue, 70),
    (FlowerColor::Purple, 70),
    (FlowerColor::Azure, 70),
    (FlowerColor::Pink, 60),
    (FlowerColor::Glow, 30),
];
pub(super) const FLOWER_COLOR_WEIGHTS_TOTAL: u32 = 1000;
const _: () = assert!(
    weight_table_sum(&FLOWER_COLOR_WEIGHTS) == FLOWER_COLOR_WEIGHTS_TOTAL,
    "FLOWER_COLOR_WEIGHTS rows must sum to FLOWER_COLOR_WEIGHTS_TOTAL",
);

const FLOWER_CORE_WEIGHTS: [(FlowerCore, u16); 3] = [
    (FlowerCore::White, 495),
    (FlowerCore::Yellow, 495),
    (FlowerCore::Black, 10),
];
const FLOWER_CORE_WEIGHTS_TOTAL: u32 = 1000;
const _: () = assert!(
    weight_table_sum(&FLOWER_CORE_WEIGHTS) == FLOWER_CORE_WEIGHTS_TOTAL,
    "FLOWER_CORE_WEIGHTS rows must sum to FLOWER_CORE_WEIGHTS_TOTAL",
);

const CORE_EDGE_WEIGHTS: [(CoreEdge, u16); 3] = [
    (CoreEdge::White, 70),
    (CoreEdge::MatchPetalCenter, 15),
    (CoreEdge::MatchPetalEdge, 15),
];
const CORE_EDGE_WEIGHTS_TOTAL: u32 = 100;
const _: () = assert!(
    weight_table_sum(&CORE_EDGE_WEIGHTS) == CORE_EDGE_WEIGHTS_TOTAL,
    "CORE_EDGE_WEIGHTS rows must sum to CORE_EDGE_WEIGHTS_TOTAL",
);

const PETAL_COUNT_WEIGHTS: [(u8, u16); 4] = [
    (5, 9939),
    (6, 50),
    (7, 10),
    (8, 1),
];
const PETAL_COUNT_WEIGHTS_TOTAL: u32 = 10000;
const _: () = assert!(
    weight_table_sum(&PETAL_COUNT_WEIGHTS) == PETAL_COUNT_WEIGHTS_TOTAL,
    "PETAL_COUNT_WEIGHTS rows must sum to PETAL_COUNT_WEIGHTS_TOTAL",
);

/// Density of flowers on land: roughly one flower per N candidate
/// tiles. The presence-hash gate fires when the dimension's hash for
/// (x, y) is a multiple of this denominator.
pub const FLOWER_DENSITY_DENOM: u64 = 600;

/// A flower at a specific (x, y). Every field is derived from its own
/// hash dimension off the same (x, y) so peers compute identical
/// appearance without coordination.
///
/// 7 (petal_center) × 7 (petal_edge) × 3 (core_center) × 3 (core_edge)
/// × 4 (petal_count) = 1764 distinct kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Flower {
    pub petal_center: FlowerColor,
    pub petal_edge: FlowerColor,
    pub core_center: FlowerCore,
    pub core_edge: CoreEdge,
    pub petal_count: u8,
}

/// Deterministic flower at (x, y). Returns None for water columns,
/// polar ocean, or tiles where the presence-hash gate doesn't fire.
/// Once Some, every field of the returned Flower is fully determined —
/// no further None-checking and no defaults required by callers.
pub fn flower_at(x: i32, y: i32) -> Option<Flower> {
    if y.abs() > WORLD_Y_LAT {
        return None;
    }
    if surface_z(x, y) < 0 {
        return None;
    }
    let presence_hash = world_hash(x, y, HashDimension::FlowerPresence);
    if !presence_hash.is_multiple_of(FLOWER_DENSITY_DENOM) {
        return None;
    }
    // Petal center: an extra splitmix round on the presence hash so
    // it decorrelates from the density gate but shares its derivation.
    let petal_center = pick_weighted(
        splitmix64(presence_hash) >> 32,
        &FLOWER_COLOR_WEIGHTS,
        FLOWER_COLOR_WEIGHTS_TOTAL,
    );
    let petal_edge = pick_weighted(
        world_hash(x, y, HashDimension::FlowerPetalEdge) >> 32,
        &FLOWER_COLOR_WEIGHTS,
        FLOWER_COLOR_WEIGHTS_TOTAL,
    );
    let core_center = pick_weighted(
        world_hash(x, y, HashDimension::FlowerCoreCenter) >> 32,
        &FLOWER_CORE_WEIGHTS,
        FLOWER_CORE_WEIGHTS_TOTAL,
    );
    let core_edge = pick_weighted(
        world_hash(x, y, HashDimension::FlowerCoreEdge) >> 32,
        &CORE_EDGE_WEIGHTS,
        CORE_EDGE_WEIGHTS_TOTAL,
    );
    let petal_count = pick_weighted(
        world_hash(x, y, HashDimension::FlowerPetalCount) >> 32,
        &PETAL_COUNT_WEIGHTS,
        PETAL_COUNT_WEIGHTS_TOTAL,
    );
    Some(Flower {
        petal_center,
        petal_edge,
        core_center,
        core_edge,
        petal_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_weighted_distribution_matches_weights() {
        // Sample 100k uniform hashes and check that the empirical
        // distribution matches FLOWER_COLOR_WEIGHTS within ±2%.
        let mut counts = [0u32; 7];
        for i in 0..100_000_u64 {
            let r = splitmix64(i);
            let c = pick_weighted(r, &FLOWER_COLOR_WEIGHTS, FLOWER_COLOR_WEIGHTS_TOTAL);
            counts[c as usize] += 1;
        }
        for (color, weight) in FLOWER_COLOR_WEIGHTS {
            let observed = counts[color as usize] as f32 / 100_000.0;
            let expected = weight as f32 / FLOWER_COLOR_WEIGHTS_TOTAL as f32;
            let diff = (observed - expected).abs();
            assert!(
                diff < 0.02,
                "color {:?}: observed {:.3} expected {:.3}",
                color,
                observed,
                expected,
            );
        }
    }

    #[test]
    fn flower_at_deterministic() {
        let a = flower_at(123, -456);
        let b = flower_at(123, -456);
        assert_eq!(a, b);
    }

    #[test]
    fn flower_at_density_in_range() {
        // Over a 200x200 land sample, density should be near 1/60.
        // Allow generous slack since the gate is non-uniform per column.
        let mut flowers = 0;
        let mut land = 0;
        for x in 0..200 {
            for y in 0..200 {
                if surface_z(x, y) >= 0 {
                    land += 1;
                    if flower_at(x, y).is_some() {
                        flowers += 1;
                    }
                }
            }
        }
        let density = flowers as f32 / land as f32;
        let expected = 1.0 / FLOWER_DENSITY_DENOM as f32;
        assert!(
            (density - expected).abs() < expected,
            "flower density {density:.4} far from expected {expected:.4}"
        );
    }
}
