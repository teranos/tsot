//! Value noise + fractal sum, x-periodic for the cylindrical world.
//!
//! Used only by terrain (heightmap, break mask, cave carving). Kept in
//! its own module because the noise math is independent of any
//! particular worldgen concern.

use super::hash::{splitmix64, SPLITMIX_GAMMA};

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
//
// Per-octave seed offset uses SPLITMIX_GAMMA so consecutive octaves
// land at uncorrelated noise tables — same constant, named role.
pub(super) fn fractal_2d(seed: u64, x: f32, y: f32, octaves: u32, x_period_base: i32) -> f32 {
    let mut total = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut max_amp = 0.0;
    let mut period = x_period_base;
    for o in 0..octaves {
        let oseed = seed.wrapping_add((o as u64).wrapping_mul(SPLITMIX_GAMMA));
        total += value_noise_2d(oseed, x * freq, y * freq, period) * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
        period = period.saturating_mul(2);
    }
    total / max_amp
}

pub(super) fn fractal_3d(
    seed: u64,
    x: f32,
    y: f32,
    z: f32,
    octaves: u32,
    x_period_base: i32,
) -> f32 {
    let mut total = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut max_amp = 0.0;
    let mut period = x_period_base;
    for o in 0..octaves {
        let oseed = seed.wrapping_add((o as u64).wrapping_mul(SPLITMIX_GAMMA));
        total += value_noise_3d(oseed, x * freq, y * freq, z * freq, period) * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
        period = period.saturating_mul(2);
    }
    total / max_amp
}
