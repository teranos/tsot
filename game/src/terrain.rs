//! Terrain height — the SimCity 4 heightfield. See `docs/TERRAIN.md`.
//!
//! Slice 1: `height(x, z)` is the single source of truth for ground
//! elevation. A pure function of world XZ (same determinism model as
//! `trees::tree_at_cell`), continuous and gentle — no cliffs, no
//! mountains. Stamp flattening (Slice 2) and the draped grid (Slice 4)
//! both sample this one function.

/// Ground elevation at a world XZ position, in world units.
///
/// Two octaves of value noise over **world** coordinates. Because it
/// never touches chunk-local coordinates, the field is continuous
/// across every chunk boundary by construction. Amplitudes and
/// wavelengths are chosen so the gradient stays well under the cliff
/// threshold — gentle relief only (`docs/TERRAIN.md`, Slice 1).
pub fn height(x: f32, z: f32) -> f32 {
    // Octave 1: broad rolling relief (~one chunk wavelength).
    // Octave 2: a smaller ripple, offset so its lattice doesn't align
    // with octave 1. Both amplitudes keep max slope comfortably < 0.5.
    140.0 * value_noise(x, z, 2400.0) + 24.0 * value_noise(x + 1000.0, z - 1000.0, 900.0)
}

/// Deterministic pseudo-random value in `[-1, 1]` for an integer
/// lattice point. Integer hash — no float state, fully reproducible.
fn hash2(ix: i32, iz: i32) -> f32 {
    let mut h =
        (ix as u32).wrapping_mul(0x9E37_79B1) ^ (iz as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Smoothstep fade — gives the interpolation a zero-slope join at each
/// lattice line, so the field is C1 and has no kinks to read as seams.
fn fade(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear value noise at `wavelength` world units per lattice cell.
/// Returns `[-1, 1]`.
fn value_noise(x: f32, z: f32, wavelength: f32) -> f32 {
    let u = x / wavelength;
    let v = z / wavelength;
    let u0 = u.floor();
    let v0 = v.floor();
    let (ix, iz) = (u0 as i32, v0 as i32);
    let (sx, sz) = (fade(u - u0), fade(v - v0));

    let n00 = hash2(ix, iz);
    let n10 = hash2(ix + 1, iz);
    let n01 = hash2(ix, iz + 1);
    let n11 = hash2(ix + 1, iz + 1);

    let nx0 = n00 + (n10 - n00) * sx;
    let nx1 = n01 + (n11 - n01) * sx;
    nx0 + (nx1 - nx0) * sz
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rise/run cap. Gentle relief only — a slope above this reads as a
    /// cliff, which is a locked non-goal, and is also how a per-chunk
    /// seam tear shows up (a huge jump over a tiny step).
    const SLOPE_CAP: f32 = 0.5;
    /// The field must actually have relief across a few chunks — this
    /// many world units of variation, minimum. Kills a flat stub.
    const MIN_RELIEF: f32 = 50.0;
    /// Small horizontal step for the slope check.
    const STEP: f32 = 1.0;

    #[test]
    fn height_is_a_deterministic_gentle_continuous_field() {
        use crate::chunk::CHUNK_SIZE;

        // 1) Deterministic — same input, same output, every call.
        for &(x, z) in &[
            (0.0, 0.0),
            (137.0, -412.5),
            (CHUNK_SIZE, CHUNK_SIZE),
            (-9001.0, 3210.0),
        ] {
            assert_eq!(height(x, z), height(x, z), "height not deterministic at ({x},{z})");
        }

        // 2) Relief exists — sample a few chunks wide, assert it is not
        //    flat (and never NaN/inf).
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut x = -3.0 * CHUNK_SIZE;
        while x <= 3.0 * CHUNK_SIZE {
            let mut z = -3.0 * CHUNK_SIZE;
            while z <= 3.0 * CHUNK_SIZE {
                let h = height(x, z);
                assert!(h.is_finite(), "height not finite at ({x},{z}): {h}");
                lo = lo.min(h);
                hi = hi.max(h);
                z += 60.0;
            }
            x += 60.0;
        }
        assert!(hi - lo > MIN_RELIEF, "field is flat: relief {} <= {MIN_RELIEF}", hi - lo);

        // 3) Continuous + gentle — a small step never produces a big
        //    jump, INCLUDING across exact chunk boundaries (the place a
        //    chunk-local implementation tears). Each base straddles a
        //    seam or sits in open field; we step across it in x and z.
        let bases = [
            (0.0, 0.0),
            (500.0, -1200.0),
            (-2000.0, 800.0),
            (CHUNK_SIZE, 300.0),           // x seam
            (CHUNK_SIZE, -CHUNK_SIZE),     // x and z seam corner
            (-2.0 * CHUNK_SIZE, CHUNK_SIZE),
        ];
        for &(bx, bz) in &bases {
            for (ax, az, cx, cz) in [
                (bx - STEP / 2.0, bz, bx + STEP / 2.0, bz), // across x
                (bx, bz - STEP / 2.0, bx, bz + STEP / 2.0), // across z
            ] {
                let slope = (height(cx, cz) - height(ax, az)).abs() / STEP;
                assert!(
                    slope <= SLOPE_CAP,
                    "slope {slope} exceeds {SLOPE_CAP} near ({bx},{bz}) — cliff or seam tear"
                );
            }
        }
    }
}
