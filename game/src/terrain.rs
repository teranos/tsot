//! Terrain height — the SimCity 4 heightfield. See `docs/TERRAIN.md`.
//!
//! Slice 1: `height(x, z)` is the single source of truth for ground
//! elevation. A pure function of world XZ (same determinism model as
//! `trees::tree_at_cell`), continuous and gentle — no cliffs, no
//! mountains. Stamp flattening (Slice 2) and the draped grid (Slice 4)
//! both sample this one function.

/// Ground elevation at a world XZ position, in world units. The single
/// source of truth — terrain mesh, draped grid, props and player all
/// sample this.
///
/// Base relief is two octaves of value noise over **world** coordinates,
/// so it's continuous across every chunk boundary by construction and
/// gentle by amplitude (`docs/TERRAIN.md`, Slice 1). Inside a CDDA stamp
/// footprint the field is flattened to the stamp's pad level, in its
/// entirety (Slice 2).
pub fn height(x: f32, z: f32) -> f32 {
    let base = base_height(x, z);
    let s = stamps();
    if s.num == 0 {
        return base;
    }
    use crate::chunk::CHUNK_SIZE;
    let qx = (x / CHUNK_SIZE).floor() as i32;
    let qz = (z / CHUNK_SIZE).floor() as i32;
    // Search every chunk whose stamp footprint (plus its skirt) could
    // reach here. A multi-tile stamp (the school) spans several chunks,
    // so `reach` is sized to the largest footprint — the pad is respected
    // in its entirety, never clipped at a chunk edge.
    //
    // Inside a footprint → the flat pad. Just outside → a skirt that ramps
    // the pad level to the surrounding relief, so the join is continuous.
    // We track the nearest pad edge; the pad closest to the point governs
    // its skirt.
    let mut skirt: Option<(f32, f32)> = None; // (pad level, t: 0 at edge → 1 far)
    for dx in -s.reach..=s.reach {
        for dz in -s.reach..=s.reach {
            let (cx, cz) = (qx + dx, qz + dz);
            let Some(anchor) = cdda::building_anchor_in_chunk(cx, cz, CHUNK_SIZE) else {
                continue;
            };
            let idx = cdda::building_index(cx, cz, s.num);
            let ph = s.pad_half[idx];
            // Chebyshev (square) distance from the anchor, matching the
            // square footprint.
            let cheb = (x - anchor.x).abs().max((z - anchor.z).abs());
            if cheb <= ph {
                // Flat pad, exact — the whole footprint sits at the ground
                // level of the stamp's anchor.
                //
                // NOTE: the flat area is the full authored footprint
                // INCLUDING the yard — `pad_half` folds the yard clearance
                // in. This is a for-now choice.
                return base_height(anchor.x, anchor.z);
            }
            let out = cheb - ph;
            if out < SKIRT {
                let t = out / SKIRT;
                if skirt.map_or(true, |(_, pt)| t < pt) {
                    skirt = Some((base_height(anchor.x, anchor.z), t));
                }
            }
        }
    }
    match skirt {
        // fade(0)=0 → pad level at the edge (continuous with the pad);
        // fade(1)=1 → base relief at the skirt's outer rim (continuous
        // with open terrain). Smoothstep keeps the ramp gentle.
        Some((pad, t)) => pad + (base - pad) * fade(t),
        None => base,
    }
}

/// How far the skirt ramps a pad level out to the surrounding relief,
/// in world units. Wide enough that even a full-relief drop stays a
/// gentle slope, never a cliff.
const SKIRT: f32 = 1500.0;

/// Cached building-footprint data, loaded once. Flattening needs the
/// per-template pad half-extent and how far a footprint can reach.
struct Stamps {
    num: usize,
    /// Per template: the flat pad's square half-extent — the larger of
    /// the yard clearance and the authored prop/tree reach.
    pad_half: Vec<f32>,
    /// Chunk rings a footprint can span from its anchor chunk.
    reach: i32,
}

fn stamps() -> &'static Stamps {
    static CELL: std::sync::OnceLock<Stamps> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        // Import failures are surfaced by the primary load at startup
        // (`buildings::BuildingTemplates::load` → obs, `lib.rs` init);
        // this second load is geometry-only. An empty set → num 0 → no
        // flattening, which shows up as absent pads in the render.
        let (bt, _failures) = cdda::load_building_templates();
        let pad_half: Vec<f32> = bt
            .half_extents
            .iter()
            .map(|&h| cdda::BUILDING_FOOTPRINT_HALF.max(h))
            .collect();
        let max_half = pad_half.iter().copied().fold(0.0_f32, f32::max);
        let reach =
            ((max_half + crate::chunk::CHUNK_SIZE * 0.5) / crate::chunk::CHUNK_SIZE).ceil() as i32;
        Stamps { num: bt.templates.len(), pad_half, reach }
    })
}

/// The bare relief field, before stamp flattening — two octaves of value
/// noise over world coordinates.
fn base_height(x: f32, z: f32) -> f32 {
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
    fn cdda_stamp_footprint_is_flat_in_its_entirety() {
        use crate::chunk::CHUNK_SIZE;

        let (bt, _failures) = cdda::load_building_templates();
        let num = bt.templates.len();
        assert!(num > 0, "no building templates loaded — cdda corpus missing?");

        // School = the largest authored footprint (same pick the seer
        // tour makes). Full flat area = the larger of the tree-yard
        // clearance and the authored prop/tree reach — "incl. yard".
        let school = bt
            .half_extents
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap();
        let pad_half = cdda::BUILDING_FOOTPRINT_HALF.max(bt.half_extents[school]);

        // Nearest chunk that actually hosts the school.
        let anchor = (1..400i32)
            .find_map(|r| {
                let mut hit = None;
                for x in -r..=r {
                    for z in -r..=r {
                        if x.abs() != r && z.abs() != r {
                            continue; // ring only
                        }
                        if cdda::building_anchor_in_chunk(x, z, CHUNK_SIZE).is_some()
                            && cdda::building_index(x, z, num) == school
                        {
                            hit = cdda::building_anchor_in_chunk(x, z, CHUNK_SIZE);
                        }
                    }
                }
                hit
            })
            .expect("no school stamp found within scan radius");

        let pad = height(anchor.x, anchor.z);

        // Every point in the full footprint — including corners that may
        // fall into neighbouring chunks — is the SAME height. Flat in its
        // entirety; the lookup must cover the whole stamp, not just the
        // anchor chunk.
        let n = 32;
        for i in 0..=n {
            for j in 0..=n {
                let x = anchor.x - pad_half + 2.0 * pad_half * (i as f32 / n as f32);
                let z = anchor.z - pad_half + 2.0 * pad_half * (j as f32 / n as f32);
                assert!(
                    (height(x, z) - pad).abs() < 1e-3,
                    "pad not flat at ({x:.0},{z:.0}): {:.3} vs pad {pad:.3}",
                    height(x, z)
                );
            }
        }

        // Relief resumes outside the pad — flattening didn't eat the world.
        let outside_varies = [(-2.0, -2.0), (2.0, -2.0), (-2.0, 2.0), (2.0, 2.0), (3.0, 0.0), (0.0, 3.0)]
            .iter()
            .any(|(dx, dz)| {
                (height(anchor.x + dx * pad_half, anchor.z + dz * pad_half) - pad).abs() > 1.0
            });
        assert!(outside_varies, "no relief outside the pad");
    }

    /// Nearest school stamp: (anchor x, anchor z, pad half-extent).
    fn nearest_school() -> (f32, f32, f32) {
        use crate::chunk::CHUNK_SIZE;
        let (bt, _f) = cdda::load_building_templates();
        let num = bt.templates.len();
        assert!(num > 0, "no building templates — cdda corpus missing?");
        let school = bt
            .half_extents
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap();
        let ph = cdda::BUILDING_FOOTPRINT_HALF.max(bt.half_extents[school]);
        let a = (1..400i32)
            .find_map(|r| {
                let mut hit = None;
                for x in -r..=r {
                    for z in -r..=r {
                        if x.abs() != r && z.abs() != r {
                            continue;
                        }
                        if cdda::building_anchor_in_chunk(x, z, CHUNK_SIZE).is_some()
                            && cdda::building_index(x, z, num) == school
                        {
                            hit = cdda::building_anchor_in_chunk(x, z, CHUNK_SIZE);
                        }
                    }
                }
                hit
            })
            .expect("no school stamp found");
        (a.x, a.z, ph)
    }

    #[test]
    fn stamp_pad_edge_transitions_continuously() {
        const SLOPE_CAP: f32 = 0.5;
        let (ax, az, ph) = nearest_school();

        // Pad interior is STILL exactly flat (Slice 2 invariant holds).
        let pad = height(ax, az);
        for &(fx, fz) in &[(0.0, 0.0), (0.5, 0.5), (-0.5, 0.5), (0.9, -0.9)] {
            let (x, z) = (ax + fx * ph, az + fz * ph);
            assert!(
                (height(x, z) - pad).abs() < 1e-3,
                "pad interior not flat at ({x:.0},{z:.0})"
            );
        }

        // Crossing the footprint edge is continuous — no hard step. Walk
        // each face and step across the boundary; bounded slope, same cap
        // as open terrain. (Slice 2's hard pad→relief step fails this.)
        let d = 1.0;
        let n = 40;
        for k in 0..=n {
            let s = -ph + 2.0 * ph * (k as f32 / n as f32);
            for (ix, iz, ox, oz) in [
                (ax + ph - d / 2.0, az + s, ax + ph + d / 2.0, az + s), // +x
                (ax - ph + d / 2.0, az + s, ax - ph - d / 2.0, az + s), // -x
                (ax + s, az + ph - d / 2.0, ax + s, az + ph + d / 2.0), // +z
                (ax + s, az - ph + d / 2.0, ax + s, az - ph - d / 2.0), // -z
            ] {
                let slope = (height(ox, oz) - height(ix, iz)).abs() / d;
                assert!(
                    slope <= SLOPE_CAP,
                    "hard step across pad edge: slope {slope} > {SLOPE_CAP} at s={s:.0}"
                );
            }
        }
    }

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
