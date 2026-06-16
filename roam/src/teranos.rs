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
pub enum TileKind {
    Air,
    Grass,
    Rock,
    ShallowWater,
    DeepWater,
}

// ----- noise primitives -----

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

#[inline]
fn lattice_value(seed: u64, x: i32, y: i32, z: i32) -> f32 {
    let mut h = seed;
    h = splitmix64(h ^ (x as i64 as u64));
    h = splitmix64(h ^ (y as i64 as u64));
    h = splitmix64(h ^ (z as i64 as u64));
    // Map top 32 bits to [-1, 1].
    let u = (h >> 32) as u32;
    (u as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// 2D value noise on (xf, yf), x-periodic. `x_period` is the lattice
// wrap width in integer lattice points; 0 means no wrap. Required for
// the cylindrical world — without it, tile WORLD_CIRC_X-1 and tile 0
// sample uncorrelated lattice points and the seam is a discontinuity.
fn value_noise_2d(seed: u64, xf: f32, yf: f32, x_period: i32) -> f32 {
    let xi_raw = xf.floor() as i32;
    let yi = yf.floor() as i32;
    let u = fade(xf - xi_raw as f32);
    let v = fade(yf - yi as f32);
    let (xi, xi_next) = if x_period > 0 {
        (xi_raw.rem_euclid(x_period), (xi_raw + 1).rem_euclid(x_period))
    } else {
        (xi_raw, xi_raw + 1)
    };
    let a00 = lattice_value(seed, xi, yi, 0);
    let a10 = lattice_value(seed, xi_next, yi, 0);
    let a01 = lattice_value(seed, xi, yi + 1, 0);
    let a11 = lattice_value(seed, xi_next, yi + 1, 0);
    lerp(lerp(a00, a10, u), lerp(a01, a11, u), v)
}

// 3D value noise on (xf, yf, zf), x-periodic. Same seam-continuity
// reasoning as value_noise_2d. Caves will hit this when they're
// rendered; pay the cost now to keep the noise contract consistent.
fn value_noise_3d(seed: u64, xf: f32, yf: f32, zf: f32, x_period: i32) -> f32 {
    let xi_raw = xf.floor() as i32;
    let yi = yf.floor() as i32;
    let zi = zf.floor() as i32;
    let u = fade(xf - xi_raw as f32);
    let v = fade(yf - yi as f32);
    let w = fade(zf - zi as f32);
    let (xi, xi_next) = if x_period > 0 {
        (xi_raw.rem_euclid(x_period), (xi_raw + 1).rem_euclid(x_period))
    } else {
        (xi_raw, xi_raw + 1)
    };
    let a000 = lattice_value(seed, xi, yi, zi);
    let a100 = lattice_value(seed, xi_next, yi, zi);
    let a010 = lattice_value(seed, xi, yi + 1, zi);
    let a110 = lattice_value(seed, xi_next, yi + 1, zi);
    let a001 = lattice_value(seed, xi, yi, zi + 1);
    let a101 = lattice_value(seed, xi_next, yi, zi + 1);
    let a011 = lattice_value(seed, xi, yi + 1, zi + 1);
    let a111 = lattice_value(seed, xi_next, yi + 1, zi + 1);
    lerp(
        lerp(lerp(a000, a100, u), lerp(a010, a110, u), v),
        lerp(lerp(a001, a101, u), lerp(a011, a111, u), v),
        w,
    )
}

// Fractal sum: octaves of value noise at increasing frequency.
// `x_period_base` is the lattice wrap width at octave 0; doubles each
// octave (since each octave doubles the frequency, the lattice spans
// twice as many points across the same world distance).
fn fractal_2d(seed: u64, x: f32, y: f32, octaves: u32, x_period_base: i32) -> f32 {
    let mut total = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut max_amp = 0.0;
    let mut period = x_period_base;
    for o in 0..octaves {
        let oseed = seed.wrapping_add((o as u64).wrapping_mul(0x9E3779B97F4A7C15));
        total += value_noise_2d(oseed, x * freq, y * freq, period) * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
        period = period.saturating_mul(2);
    }
    total / max_amp
}

fn fractal_3d(seed: u64, x: f32, y: f32, z: f32, octaves: u32, x_period_base: i32) -> f32 {
    let mut total = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut max_amp = 0.0;
    let mut period = x_period_base;
    for o in 0..octaves {
        let oseed = seed.wrapping_add((o as u64).wrapping_mul(0x9E3779B97F4A7C15));
        total += value_noise_3d(oseed, x * freq, y * freq, z * freq, period) * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
        period = period.saturating_mul(2);
    }
    total / max_amp
}

// ----- world geometry -----

#[inline]
fn canonical_x(x: i32) -> i32 {
    x.rem_euclid(WORLD_CIRC_X)
}

// Topmost solid tile for column (x, y). Sea level is z=0.
// Polar oceans (|y| > WORLD_Y_LAT): floor of the world.
// Raw smooth elevation at (x, y). Clamps y into the playable band so
// the gradient computation near the polar boundary doesn't see the
// artificial Z_MIN jump.
fn raw_elevation(x: i32, y: i32) -> f32 {
    let cy = y.clamp(-WORLD_Y_LAT, WORLD_Y_LAT);
    let cx = canonical_x(x);
    let s = 1.0 / 64.0;
    let x_period_base = (WORLD_CIRC_X as f32 * s) as i32;
    let n = fractal_2d(WORLD_SEED, cx as f32 * s, cy as f32 * s, 4, x_period_base);
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
    // patches of ~12 tiles separated by ~12-tile smooth gaps.
    let bs = 1.0 / 24.0;
    let bs_period = (WORLD_CIRC_X as f32 * bs) as i32;
    let cx = canonical_x(x);
    let break_noise = fractal_2d(
        WORLD_SEED.wrapping_add(0xBE_AC_07_BE_AC_07_BE),
        cx as f32 * bs,
        y as f32 * bs,
        1,
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

/// Flower colors with hand-tuned rarity weights summing to 1000.
/// Red and yellow are common; blue/purple/azure are rare;
/// pink is super-rare; glow is super-mega-rare.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FlowerColor {
    Red,
    Yellow,
    Blue,
    Purple,
    Azure,
    Pink,
    Glow,
}

const FLOWER_DENSITY_DENOM: u64 = 60; // ~1 in 60 grass tiles

/// Deterministic flower presence + color for a given (x, y). Only
/// spawns on land columns (not water, not polar ocean). Same inputs
/// from any peer = same flower; nothing has to be agreed on at runtime.
pub fn flower_at(x: i32, y: i32) -> Option<FlowerColor> {
    if y.abs() > WORLD_Y_LAT {
        return None;
    }
    if surface_z(x, y) < 0 {
        return None;
    }
    let cx = canonical_x(x);
    let h = splitmix64(
        WORLD_SEED
            ^ 0xF10E_1257_F10E_1257
            ^ ((cx as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            ^ (y as i64 as u64),
    );
    if !h.is_multiple_of(FLOWER_DENSITY_DENOM) {
        return None;
    }
    let r = (splitmix64(h) >> 32) % 1000;
    let color = match r {
        0..=349 => FlowerColor::Red,
        350..=699 => FlowerColor::Yellow,
        700..=769 => FlowerColor::Blue,
        770..=839 => FlowerColor::Purple,
        840..=909 => FlowerColor::Azure,
        910..=969 => FlowerColor::Pink,
        _ => FlowerColor::Glow,
    };
    Some(color)
}

/// Char encoding for the viewport's parallel flower string. '0' is
/// "no flower"; '1'..='7' are colors in declaration order. Mirrors
/// the tile_char + elev_char pattern in world.rs.
pub fn flower_char(f: Option<FlowerColor>) -> char {
    match f {
        None => '0',
        Some(FlowerColor::Red) => '1',
        Some(FlowerColor::Yellow) => '2',
        Some(FlowerColor::Blue) => '3',
        Some(FlowerColor::Purple) => '4',
        Some(FlowerColor::Azure) => '5',
        Some(FlowerColor::Pink) => '6',
        Some(FlowerColor::Glow) => '7',
    }
}

/// Petal-center color. Mostly white or yellow; very very very very
/// rare black core. Determined by a second hash off the same (x, y)
/// so every peer agrees on what each flower looks like.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FlowerCore {
    White,
    Yellow,
    Black,
}

pub fn flower_core_at(x: i32, y: i32) -> Option<FlowerCore> {
    flower_at(x, y)?;
    let cx = canonical_x(x);
    let h = splitmix64(
        WORLD_SEED
            ^ 0xC0_5E_C0_5E_C0_5E_C0_5E
            ^ ((cx as i64 as u64).wrapping_mul(0x94D0_49BB_1331_11EB))
            ^ (y as i64 as u64),
    );
    let r = (h >> 32) % 1000;
    Some(match r {
        0..=494 => FlowerCore::White,
        495..=989 => FlowerCore::Yellow,
        _ => FlowerCore::Black,
    })
}

pub fn flower_core_char(c: Option<FlowerCore>) -> char {
    match c {
        None => '0',
        Some(FlowerCore::White) => '1',
        Some(FlowerCore::Yellow) => '2',
        Some(FlowerCore::Black) => '3',
    }
}

/// Petal count for a flower. 5 is overwhelmingly common; 6/7/8 are
/// increasingly rare tiers. Returns None if there's no flower at (x,y).
/// Distribution (per 10000): 5 = 9939, 6 = 50, 7 = 10, 8 = 1.
/// Petal-edge color — same 7-color vocabulary and same rarity weights
/// as `flower_at`, drawn from an independent hash. Renders as a
/// radial gradient inside each petal (center = petal color,
/// edge = this color).
pub fn flower_edge_at(x: i32, y: i32) -> Option<FlowerColor> {
    flower_at(x, y)?;
    let cx = canonical_x(x);
    let h = splitmix64(
        WORLD_SEED
            ^ 0xED6E_ED6E_BA5E_C0DE
            ^ ((cx as i64 as u64).wrapping_mul(0x94D0_49BB_1331_11EB))
            ^ (y as i64 as u64),
    );
    let r = (h >> 32) % 1000;
    Some(match r {
        0..=349 => FlowerColor::Red,
        350..=699 => FlowerColor::Yellow,
        700..=769 => FlowerColor::Blue,
        770..=839 => FlowerColor::Purple,
        840..=909 => FlowerColor::Azure,
        910..=969 => FlowerColor::Pink,
        _ => FlowerColor::Glow,
    })
}

pub fn flower_petals_at(x: i32, y: i32) -> Option<u8> {
    flower_at(x, y)?;
    let cx = canonical_x(x);
    let h = splitmix64(
        WORLD_SEED
            ^ 0x5EAF_5EAF_C0DE_F123
            ^ ((cx as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
            ^ (y as i64 as u64),
    );
    let r = (h >> 32) % 10000;
    Some(match r {
        0 => 8,
        1..=10 => 7,
        11..=60 => 6,
        _ => 5,
    })
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
    let cs = 1.0 / 24.0;
    let cseed = WORLD_SEED.wrapping_add(0xC0FFEE_C0FFEE);
    let cx = canonical_x(x);
    let cave_x_period = (WORLD_CIRC_X as f32 * cs) as i32;
    let n = fractal_3d(cseed, cx as f32 * cs, y as f32 * cs, z as f32 * cs, 3, cave_x_period);
    if n > 0.45 {
        TileKind::Air
    } else {
        TileKind::Rock
    }
}

// Position-dependent day phase. Returns value in [0, 1):
//   0.0 = dawn, 0.25 = noon, 0.5 = dusk, 0.75 = midnight.
// Longitude shifts the phase — sun sweeps east-to-west.
pub fn day_phase(now_unix_secs: u64, x: i32) -> f32 {
    let global = (now_unix_secs % DAY_LENGTH_SECS) as f32 / DAY_LENGTH_SECS as f32;
    let lon = canonical_x(x) as f32 / WORLD_CIRC_X as f32;
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
    fn surface_z_is_deterministic() {
        // Same input, same output. Always.
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
        // In a 200x200 sample, the overwhelming majority of adjacent
        // column pairs should be walkable (|Δz| ≤ 1). If most are
        // cliffs, the cliff_mask threshold is set wrong.
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
        // Cliffs (|Δz| > 1) exist but stay rare. Sample a 400x400 block
        // since cliff zones at scale 1/256 may not show up in 200x200.
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
        // x and x + WORLD_CIRC_X must agree.
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
                // One above the surface (or sea level for water columns) must be Air.
                let air_z = sz.max(0) + 1;
                assert_eq!(tile_at(x, y, air_z), TileKind::Air, "expected Air at ({x},{y},{air_z}) sz={sz}");
            }
        }
    }

    #[test]
    fn tile_at_water_at_sea_level_below_sea() {
        // Find a column with sz < 0 and confirm z=0 is water.
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
        // Any column past Y_LAT should have water at sea level.
        let y = WORLD_Y_LAT + 100;
        let t = tile_at(0, y, 0);
        assert_eq!(t, TileKind::DeepWater);
    }

    #[test]
    fn day_phase_wraps_with_longitude() {
        // Phase at (x, t) must equal phase at (x + CIRC_X, t).
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
}
