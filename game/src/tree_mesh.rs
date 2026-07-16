//! Pure geometry for trees on the mesh pipeline. No GPU, no Bevy, no
//! browser — a tree becomes a vertex+index list plus a set of
//! phyllotactic canopy stations, deterministic in the inputs. The
//! pipeline plumbing (not yet built) consumes what this emits.
//!
//! The tree is the *vehicle* for landing the mesh pipeline; walls are
//! the north star (see `game/docs/RENDER.md`). Every design call in
//! this module is made so a wall / flower / terrain generator can
//! reuse the same vertex format and pipeline unchanged.

/// One vertex on the mesh pipeline. `uv` is here from day one even
/// though tree materials don't sample textures yet — damage textures
/// / posters / brick are downstream goals, and rewriting every
/// vertex format later is a second swap we're refusing to pay
/// (see `game/docs/RENDER.md`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// One phyllotactic placement around the growth axis. `angle` is
/// radians from +X toward +Z (right-handed around +Y). `radius` is
/// distance from the trunk axis in world units. `height_frac` is the
/// vertical position within the canopy (0 = crown base, 1 = tip).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanopyStation {
    pub angle: f32,
    pub radius: f32,
    pub height_frac: f32,
}

/// The golden angle in radians: 2π × (1 − 1/φ), where φ = (1+√5)/2.
/// ≈ 137.5077640500378° ≈ 2.399963229728653 rad. The "most
/// irrational" rotation — successive rotations by this amount never
/// realign, so phyllotactic placements pack the growth surface
/// without radial overlap at any density. See sunflower discs,
/// pinecones, pineapple spirals.
pub const GOLDEN_ANGLE_RAD: f32 = 2.399_963_2;

/// Trunk as a truncated open cone: `sides` vertical facets, tapering
/// from `base_radius` at y=0 to `top_radius` at y=height. No end
/// caps — the trunk bottom sits at ground, the canopy covers the
/// top from above. Rings are CCW when viewed from +Y so
/// front-face-CCW backface culling shows the outside.
///
/// Layout of returned vertices: `[base_ring (0..sides)]` then
/// `[top_ring (sides..2*sides)]`. Indices form `sides` side quads
/// as `2 * sides` CCW triangles = `6 * sides` indices.
pub fn trunk_mesh(
    sides: u32,
    base_radius: f32,
    top_radius: f32,
    height: f32,
) -> (Vec<MeshVertex>, Vec<u32>) {
    let n = sides as usize;
    let mut verts = Vec::with_capacity(2 * n);
    let dr = top_radius - base_radius;
    // Normal slope: for a tapered cone the true outward surface normal
    // tilts upward by atan(-dr / height) — narrower-at-top surfaces
    // reflect light as though facing slightly skyward, which shades a
    // trunk correctly instead of ignoring the taper. See derivation in
    // the module head: normal ∝ (height·cos, −dr, height·sin).
    let normal_mag = (height * height + dr * dr).sqrt().max(f32::EPSILON);
    let ny = -dr / normal_mag;
    let n_horiz = height / normal_mag;
    for i in 0..n {
        let theta = (i as f32) * std::f32::consts::TAU / (n as f32);
        let (s, c) = theta.sin_cos();
        let nx = n_horiz * c;
        let nz = n_horiz * s;
        // UVs wrap once around theta at v=0 (base) and v=1 (top). Even
        // spacing so a brick texture tiles cleanly across the trunk.
        let u = (i as f32) / (n as f32);
        verts.push(MeshVertex {
            pos: [base_radius * c, 0.0, base_radius * s],
            normal: [nx, ny, nz],
            uv: [u, 0.0],
        });
    }
    for i in 0..n {
        let theta = (i as f32) * std::f32::consts::TAU / (n as f32);
        let (s, c) = theta.sin_cos();
        let nx = n_horiz * c;
        let nz = n_horiz * s;
        let u = (i as f32) / (n as f32);
        verts.push(MeshVertex {
            pos: [top_radius * c, height, top_radius * s],
            normal: [nx, ny, nz],
            uv: [u, 1.0],
        });
    }
    // Side quads: two CCW-outward triangles per facet. Winding is
    // (base[i], top[i], base[i+1]) then (base[i+1], top[i], top[i+1]),
    // both producing normals in the +radial direction when the base
    // ring is oriented CCW-from-above (which it is: cos/sin of
    // increasing theta traces +X → +Z → −X → −Z, i.e. CCW in the
    // right-handed +Y-up frame). Front-face-CCW backface culling then
    // shows the outside of the trunk.
    let mut indices = Vec::with_capacity(6 * n);
    for i in 0..n {
        let a = i as u32;
        let b = ((i + 1) % n) as u32;
        let a_top = (i + n) as u32;
        let b_top = ((i + 1) % n + n) as u32;
        indices.push(a);
        indices.push(a_top);
        indices.push(b);
        indices.push(b);
        indices.push(a_top);
        indices.push(b_top);
    }
    (verts, indices)
}

/// Place `count` canopy elements around the trunk axis at successive
/// golden-angle rotations. Radii grow as `canopy_radius * √(n/count)`
/// (sunflower packing — dense near the trunk, thinning at the
/// periphery). Height fractions distribute through [0, 1] so the
/// crown has vertical volume, not just a disc.
/// Small spherical element placed at every canopy station. A unit
/// icosahedron (12 verts, 20 tris) — smooth-shaded via radial
/// normals since a vertex on the unit sphere IS its own normal.
/// Baked once per program; every canopy element on every tree is
/// one instanced draw of this shared geometry, transformed by the
/// per-instance world position + colour + scale.
///
/// Coordinates use the standard `(±1, ±φ, 0)` permutation, scaled
/// to unit length by dividing by `√(1 + φ²)`. Index triples are the
/// canonical icosahedron winding, oriented CCW as seen from outside
/// so front-face-CCW backface culling shows the outside.
pub fn canopy_element_mesh() -> (Vec<MeshVertex>, Vec<u32>) {
    let phi = (1.0_f32 + 5.0_f32.sqrt()) * 0.5;
    let inv = 1.0 / (1.0 + phi * phi).sqrt();
    let p = phi * inv;
    let q = inv;
    let raw: [[f32; 3]; 12] = [
        [-q,  p,  0.0], [ q,  p,  0.0], [-q, -p,  0.0], [ q, -p,  0.0],
        [ 0.0, -q,  p], [ 0.0,  q,  p], [ 0.0, -q, -p], [ 0.0,  q, -p],
        [ p,  0.0, -q], [ p,  0.0,  q], [-p,  0.0, -q], [-p,  0.0,  q],
    ];
    let verts = raw
        .into_iter()
        .map(|pos| MeshVertex {
            pos,
            normal: pos,
            uv: [0.5, 0.5],
        })
        .collect();
    let indices: Vec<u32> = vec![
        0, 11, 5,   0, 5, 1,    0, 1, 7,    0, 7, 10,   0, 10, 11,
        1, 5, 9,    5, 11, 4,   11, 10, 2,  10, 7, 6,   7, 1, 8,
        3, 9, 4,    3, 4, 2,    3, 2, 6,    3, 6, 8,    3, 8, 9,
        4, 9, 5,    2, 4, 11,   6, 2, 10,   8, 6, 7,    9, 8, 1,
    ];
    (verts, indices)
}

pub fn canopy_stations(count: u32, canopy_radius: f32) -> Vec<CanopyStation> {
    let n = count as usize;
    let mut stations = Vec::with_capacity(n);
    // Normalize so station 0 sits at radius 0 (at the trunk) and
    // station (count-1) sits at exactly `canopy_radius`. `denom` is
    // (count-1) to make that last-station equality exact; if count==1
    // we fall back to placing the sole station on-axis.
    let denom = ((n.saturating_sub(1)) as f32).max(1.0);
    for i in 0..n {
        let frac = (i as f32) / denom;
        stations.push(CanopyStation {
            angle: (i as f32) * GOLDEN_ANGLE_RAD,
            radius: canopy_radius * frac.sqrt(),
            height_frac: frac,
        });
    }
    stations
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn radius_xz(v: &MeshVertex) -> f32 {
        (v.pos[0] * v.pos[0] + v.pos[2] * v.pos[2]).sqrt()
    }

    #[test]
    fn golden_angle_matches_derivation() {
        // 2π (1 − 1/φ) computed at test time, checked against the const.
        let phi = (1.0_f64 + 5.0_f64.sqrt()) * 0.5;
        let expected = std::f64::consts::TAU * (1.0 - 1.0 / phi);
        assert!(
            (GOLDEN_ANGLE_RAD as f64 - expected).abs() < 1e-5,
            "GOLDEN_ANGLE_RAD={} but 2π(1−1/φ)={}",
            GOLDEN_ANGLE_RAD,
            expected
        );
    }

    #[test]
    fn trunk_topology_is_two_rings_no_caps() {
        let (verts, indices) = trunk_mesh(12, 5.0, 3.0, 100.0);
        assert_eq!(verts.len(), 24, "12 sides × 2 rings = 24 verts (no caps)");
        assert_eq!(
            indices.len(),
            72,
            "12 side quads × 2 tris × 3 idx = 72 indices"
        );
        for &i in &indices {
            assert!((i as usize) < verts.len(), "index {i} out of range");
        }
        assert_eq!(indices.len() % 3, 0, "indices must form whole triangles");
    }

    #[test]
    fn trunk_base_ring_at_y0_top_ring_at_height() {
        let (verts, _) = trunk_mesh(12, 5.0, 3.0, 100.0);
        for v in &verts[..12] {
            assert!(v.pos[1].abs() < EPS, "base ring y should be 0, got {}", v.pos[1]);
        }
        for v in &verts[12..24] {
            assert!(
                (v.pos[1] - 100.0).abs() < EPS,
                "top ring y should be height, got {}",
                v.pos[1]
            );
        }
    }

    #[test]
    fn trunk_is_tapered() {
        let (verts, _) = trunk_mesh(12, 5.0, 3.0, 100.0);
        for v in &verts[..12] {
            assert!(
                (radius_xz(v) - 5.0).abs() < EPS,
                "base ring radius should be 5.0, got {}",
                radius_xz(v)
            );
        }
        for v in &verts[12..24] {
            assert!(
                (radius_xz(v) - 3.0).abs() < EPS,
                "top ring radius should be 3.0, got {}",
                radius_xz(v)
            );
        }
    }

    #[test]
    fn trunk_side_normals_point_outward() {
        // For every side vertex, the horizontal component of its normal
        // must point roughly away from the trunk axis (dot(normal_xz,
        // pos_xz) > 0). A radially-inward normal would render the
        // trunk lit from the inside — a common cone-generation bug.
        let (verts, _) = trunk_mesh(12, 5.0, 3.0, 100.0);
        for v in &verts {
            let dot = v.normal[0] * v.pos[0] + v.normal[2] * v.pos[2];
            assert!(
                dot > 0.0,
                "side normal points inward at pos={:?} normal={:?}",
                v.pos,
                v.normal
            );
        }
    }

    #[test]
    fn canopy_produces_requested_count() {
        assert_eq!(canopy_stations(20, 30.0).len(), 20);
        assert_eq!(canopy_stations(64, 30.0).len(), 64);
    }

    #[test]
    fn canopy_stations_step_by_golden_angle() {
        let stations = canopy_stations(50, 30.0);
        for pair in stations.windows(2) {
            let raw = pair[1].angle - pair[0].angle;
            let delta = raw.rem_euclid(std::f32::consts::TAU);
            let expected = GOLDEN_ANGLE_RAD.rem_euclid(std::f32::consts::TAU);
            assert!(
                (delta - expected).abs() < 1e-3,
                "consecutive angle delta {delta} != golden {expected}"
            );
        }
    }

    #[test]
    fn canopy_radii_grow_monotonically() {
        // Sunflower packing: r_n ∝ √n. Non-decreasing in n, spans the
        // full canopy — first station near the trunk, last near
        // canopy_radius.
        let cr = 30.0_f32;
        let stations = canopy_stations(50, cr);
        for pair in stations.windows(2) {
            assert!(
                pair[1].radius >= pair[0].radius - EPS,
                "radii should be non-decreasing, got {} then {}",
                pair[0].radius,
                pair[1].radius
            );
        }
        let last = stations.last().unwrap().radius;
        assert!(
            (last - cr).abs() < EPS,
            "last station radius should equal canopy_radius, got {last} vs {cr}"
        );
    }

    #[test]
    fn canopy_height_fractions_span_zero_to_one() {
        let stations = canopy_stations(64, 30.0);
        let min = stations.iter().map(|s| s.height_frac).fold(f32::INFINITY, f32::min);
        let max = stations.iter().map(|s| s.height_frac).fold(f32::NEG_INFINITY, f32::max);
        assert!(min >= 0.0 - EPS, "height_frac below 0: {min}");
        assert!(max <= 1.0 + EPS, "height_frac above 1: {max}");
        assert!(
            max - min > 0.5,
            "canopy should have vertical volume, height span was {}",
            max - min
        );
    }

    #[test]
    fn canopy_element_topology_is_icosahedron() {
        let (verts, indices) = canopy_element_mesh();
        assert_eq!(verts.len(), 12, "icosahedron has 12 vertices");
        assert_eq!(indices.len(), 60, "20 triangles × 3 indices");
        for &i in &indices {
            assert!((i as usize) < verts.len(), "index {i} out of range");
        }
    }

    #[test]
    fn canopy_element_verts_lie_on_unit_sphere() {
        let (verts, _) = canopy_element_mesh();
        for v in &verts {
            let r = (v.pos[0].powi(2) + v.pos[1].powi(2) + v.pos[2].powi(2)).sqrt();
            assert!((r - 1.0).abs() < 1e-5, "vertex not on unit sphere: r={r}");
        }
    }

    #[test]
    fn canopy_element_faces_are_outward() {
        // For every triangle, the face normal (cross of two edges) must
        // point in the same hemisphere as the face centroid — i.e. away
        // from the sphere centre. Guards against a mistranscribed index
        // table that would flip winding on any face.
        let (verts, indices) = canopy_element_mesh();
        for tri in indices.chunks_exact(3) {
            let a = verts[tri[0] as usize].pos;
            let b = verts[tri[1] as usize].pos;
            let c = verts[tri[2] as usize].pos;
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let centroid = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            let dot = n[0] * centroid[0] + n[1] * centroid[1] + n[2] * centroid[2];
            assert!(
                dot > 0.0,
                "face {:?} has inward normal (dot with centroid = {dot})",
                tri
            );
        }
    }

    #[test]
    fn canopy_element_is_deterministic() {
        let (v1, i1) = canopy_element_mesh();
        let (v2, i2) = canopy_element_mesh();
        assert_eq!(v1, v2);
        assert_eq!(i1, i2);
    }

    #[test]
    fn mesh_generation_is_deterministic() {
        // Same inputs → identical bytes. Streaming + reproducibility
        // depend on this for every peer to see the same tree.
        let (v1, i1) = trunk_mesh(12, 5.0, 3.0, 100.0);
        let (v2, i2) = trunk_mesh(12, 5.0, 3.0, 100.0);
        assert_eq!(v1, v2);
        assert_eq!(i1, i2);

        let s1 = canopy_stations(20, 30.0);
        let s2 = canopy_stations(20, 30.0);
        assert_eq!(s1, s2);
    }
}
