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
    let _ = (sides, base_radius, top_radius, height);
    todo!("tree_mesh::trunk_mesh — implement to make the failing test pass")
}

/// Place `count` canopy elements around the trunk axis at successive
/// golden-angle rotations. Radii grow as `canopy_radius * √(n/count)`
/// (sunflower packing — dense near the trunk, thinning at the
/// periphery). Height fractions distribute through [0, 1] so the
/// crown has vertical volume, not just a disc.
pub fn canopy_stations(count: u32, canopy_radius: f32) -> Vec<CanopyStation> {
    let _ = (count, canopy_radius);
    todo!("tree_mesh::canopy_stations — implement to make the failing test pass")
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
