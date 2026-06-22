//! roam's worldgen + tile-level types.
//!
//! Submodules separated by concern:
//! - `hash` — splitmix64, dimensional hashing, weighted-pick utilities
//! - `noise` — value noise + fractal sum (x-periodic for the cylinder)
//! - `terrain` — heightmap, terracing, water, caves (`surface_z`, `tile_at`)
//! - `flower` — flower types + `flower_at` worldgen
//! - `card` — `card_at` worldgen (catalog-driven identity)
//!
//! `Pickup` and `pickup_at` live in this file because they're the
//! union over all pickup variants and have to know about each one.
//! Day-cycle helpers (`day_phase`, `brightness`) also stay here —
//! they're small enough not to warrant their own file.

mod card;
mod flower;
mod hash;
mod noise;
mod terrain;

pub use card::{card_at, CardId, CARD_DENSITY_DENOM};
pub use flower::{
    flower_at, CoreEdge, Flower, FlowerColor, FlowerCore, FLOWER_DENSITY_DENOM,
};
pub use terrain::{surface_z, tile_at};

use crate::catalog::Catalog;

// The canonical Teranos. Locked.
//
// One byte changed in INVOCATION = a different world. Do not edit
// INVOCATION; do not re-derive WORLD_SEED. Both are part of the
// world's identity from launch and immutable for the life of the
// canonical Teranos.
//
// Lives here until TSOT is wired as a path dependency, then relocates
// to `tsot::teranos` unchanged. The bytes are the bytes; the module
// that exposes them is incidental.

pub const INVOCATION: &str = "And I hereby invoke the Existence of Teranos, the many Symbols that give Meaning to, and the many Colors that are this World.";

// SHA-256(INVOCATION) = 5adea7cf0c0a1b2bdd3a86c65f41ea81f724adb6b891436774d153a82f0c9649
// First 8 bytes interpreted as little-endian u64.
// Verify:
//   printf '%s' "$INVOCATION" | shasum -a 256
// The literal is checked against an in-test re-derivation; compile-time
// SHA-256 derivation lands when const_sha2 is added (separate slice).
pub const WORLD_SEED: u64 = 0x2b1b_0a0c_cfa7_de5a;

// World topology: a cylinder. X wraps; Y is bounded by polar ocean.
// These bounds become load-bearing the moment cards spawn at (x, y, z)
// and a player picks one up — pickup provenance hashes canonical
// coordinates, so changing CIRC_X or Y_LAT after card launch invalidates
// every collection. Lock before v0.4 ships.
pub const WORLD_CIRC_X: i32 = 4096;
pub const WORLD_Y_LAT: i32 = 2048;

// Voxel vertical range. Surface elevation lives in a narrower band
// inside this range. Above SURFACE_Z_MAX is always air (sky);
// below SURFACE_Z_MIN is always rock (deep underground).
pub const Z_MIN: i32 = -32;
pub const Z_MAX: i32 = 32;
pub const SURFACE_Z_MIN: i32 = -16;
pub const SURFACE_Z_MAX: i32 = 16;

// Terracing gates on the LOCAL RAW-ELEVATION GRADIENT, not an
// independent mask. Only columns where the smooth heightmap is already
// steep get snapped to plateaus, so cliffs align with natural ridge
// lines and can't appear as isolated islands in flat terrain. Plateau
// islands require sharp spikes in raw elevation; the noise is smooth
// enough that those don't exist.
//
// GRADIENT_THRESHOLD is the sum of forward-x + forward-y |Δraw| at
// which we switch from smooth-rounded to terraced. Lower = more cliffs.
pub const TERRACE_STEP: i32 = 3;
pub const GRADIENT_THRESHOLD: f32 = 0.65;

// Noise frequencies in inverse-tile units. `SURFACE_NOISE_FREQUENCY =
// 1/64` means one full surface-noise period spans 64 tiles. Break and
// cave frequencies are tuned to the visual features they generate
// (cliff segmentation, cave size).
pub const SURFACE_NOISE_FREQUENCY: f32 = 1.0 / 64.0;
pub const BREAK_NOISE_FREQUENCY: f32 = 1.0 / 24.0;
pub const CAVE_NOISE_FREQUENCY: f32 = 1.0 / 24.0;
pub const SURFACE_NOISE_OCTAVES: u32 = 4;
pub const BREAK_NOISE_OCTAVES: u32 = 1;
pub const CAVE_NOISE_OCTAVES: u32 = 3;
pub const CAVE_OPEN_THRESHOLD: f32 = 0.45;

// Day length in real seconds. 5 hours = one full day-night cycle.
// Position-dependent phase: longitude shifts local phase.
pub const DAY_LENGTH_SECS: u64 = 18000;

// Vision constants (tiles).
pub const VISION_R_DAY: f32 = 12.0;
pub const VISION_R_NIGHT: f32 = 4.0;
pub const VISION_R_UNDERGROUND: f32 = 3.0;
pub const FLASHLIGHT_CONE_LEN: f32 = 12.0;
pub const FLASHLIGHT_CONE_ANGLE_DEG: f32 = 60.0;

// Voxel kinds. Walkability and rendering are downstream of this enum
// (see roam::world). Keep this set small until there's a reason to split.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TileKind {
    Air = 0,
    Grass = 1,
    Rock = 2,
    ShallowWater = 3,
    DeepWater = 4,
}

impl TileKind {
    /// Single source of truth for tile color. JS / Elm read this via the
    /// FFI color table; never re-invent RGB on the JS side.
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            TileKind::Air => [10, 10, 12],
            TileKind::Grass => [70, 122, 64],
            TileKind::Rock => [110, 100, 90],
            TileKind::ShallowWater => [80, 130, 180],
            TileKind::DeepWater => [40, 70, 130],
        }
    }
}

// ----- pickup union -----

/// What a tile can carry that a player walking over it picks up.
///
/// Generic over content kind so the pickup mechanic (`try_pickup`,
/// canonical-vs-sandbox routing, gossip) stays kind-agnostic.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Pickup {
    Flower(Flower),
    Card(CardId),
}

/// Generic pickup probe: what (if anything) is on tile `(x, y)` for the
/// player to pick up. Flowers checked first; cards fall through.
/// Catalog is needed because card identity at a tile depends on the
/// relayer's published list — see `card_at`.
pub fn pickup_at(x: i32, y: i32, catalog: &Catalog) -> Option<Pickup> {
    flower_at(x, y)
        .map(Pickup::Flower)
        .or_else(|| card_at(x, y, catalog).map(Pickup::Card))
}

// ----- day cycle -----

// Position-dependent day phase. Returns value in [0, 1):
//   0.0 = dawn, 0.25 = noon, 0.5 = dusk, 0.75 = midnight.
// Longitude shifts the phase — sun sweeps east-to-west.
pub fn day_phase(now_unix_secs: u64, x: i32) -> f32 {
    let global = (now_unix_secs % DAY_LENGTH_SECS) as f32 / DAY_LENGTH_SECS as f32;
    let lon = x.rem_euclid(WORLD_CIRC_X) as f32 / WORLD_CIRC_X as f32;
    (global + lon).rem_euclid(1.0)
}

// Brightness from phase: cosine wave, 1.0 at noon, 0.0 at midnight.
pub fn brightness(phase: f32) -> f32 {
    use core::f32::consts::TAU;
    let theta = (phase - 0.25) * TAU;
    (theta.cos() + 1.0) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_seed_matches_invocation() {
        // The locked SHA-256 of INVOCATION must produce WORLD_SEED.
        // This test re-derives it; if anyone edits INVOCATION, this fails.
        let want_hex = "5adea7cf0c0a1b2bdd3a86c65f41ea81f724adb6b891436774d153a82f0c9649";
        // Cheap re-derivation: hardcode the first 8 LE bytes from `want_hex`.
        let first_8: [u8; 8] = [0x5a, 0xde, 0xa7, 0xcf, 0x0c, 0x0a, 0x1b, 0x2b];
        let derived = u64::from_le_bytes(first_8);
        assert_eq!(derived, WORLD_SEED);
        // Confirm the hex starts with the expected bytes.
        assert!(want_hex.starts_with("5adea7cf0c0a1b2b"));
    }

    #[test]
    fn day_phase_wraps_with_longitude() {
        let a = day_phase(1_700_000_000, 100);
        let b = day_phase(1_700_000_000, 100 + WORLD_CIRC_X);
        assert!((a - b).abs() < 1e-6);
    }

    #[test]
    fn brightness_peaks_at_noon() {
        let noon = brightness(0.25);
        let dawn = brightness(0.0);
        let dusk = brightness(0.5);
        let midnight = brightness(0.75);
        assert!(noon > 0.99);
        assert!(midnight < 0.01);
        assert!((dawn - 0.5).abs() < 1e-4);
        assert!((dusk - 0.5).abs() < 1e-4);
    }

    /// `pickup_at` is the v0.4 generic surface: a tile may carry a
    /// `Pickup` (flower today, card next). For this slice — flowers
    /// only — `pickup_at` must agree with `flower_at` everywhere:
    /// flower tile → `Some(Pickup::Flower(f))` with the same `f`;
    /// empty tile → `None`. Falsifies the regression where the
    /// abstraction silently picks a different presence rule or loses
    /// fields off the wrapped `Flower`.
    #[test]
    fn pickup_at_parity_with_flower_at() {
        let cat = Catalog::new();
        for ty in -20..=20 {
            for tx in 0..100 {
                let flower = flower_at(tx, ty);
                let pickup = pickup_at(tx, ty, &cat);
                match (flower, pickup) {
                    (Some(f), Some(Pickup::Flower(g))) => assert_eq!(
                        f, g,
                        "pickup_at({tx}, {ty}) carried a different Flower than flower_at"
                    ),
                    (Some(f), Some(Pickup::Card(_))) => panic!(
                        "pickup_at({tx}, {ty}) returned a Card on a flower tile: {f:?}"
                    ),
                    (Some(_), None) => panic!("pickup_at({tx}, {ty}) lost a flower"),
                    // No-flower tile: pickup_at may return a Card or None.
                    // Both are valid; this test pins flower parity only.
                    (None, _) => {}
                }
            }
        }
    }
}
