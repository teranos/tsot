/// A solid heightfield surface — the filled, shaded ground under the
/// draped grid. A player-centred grid of `MeshVertex` (two triangles per
/// cell) sampling `height(x, z)`, with per-vertex normals from the
/// heightfield gradient so it Lambert-lights: hills catch the light,
/// valleys fall into shade. Rendered as ONE mesh, one identity instance,
/// through the mesh pipeline. See `docs/TERRAIN.md`, Slice 6.
pub struct TerrainSurface {
    pub verts: Vec<crate::tree_mesh::MeshVertex>,
    pub indices: Vec<u32>,
}

const SURF_HALF: f32 = 2400.0; // patch half-size — exceeds the view
const SURF_CELL: f32 = 150.0; // vertex spacing — resolves the fine octave

pub fn terrain_surface_mesh(px: f32, pz: f32) -> TerrainSurface {
    use crate::tree_mesh::MeshVertex;
    let n = ((2.0 * SURF_HALF) / SURF_CELL) as i32; // cells per side
    // Snap the patch origin so the surface doesn't shimmer as we move.
    let ox = (px / SURF_CELL).round() * SURF_CELL - SURF_HALF;
    let oz = (pz / SURF_CELL).round() * SURF_CELL - SURF_HALF;
    let w = n + 1; // verts per side
    let e = SURF_CELL * 0.5; // gradient sample distance
    let mut verts = Vec::with_capacity((w * w) as usize);
    for j in 0..w {
        for i in 0..w {
            let x = ox + i as f32 * SURF_CELL;
            let z = oz + j as f32 * SURF_CELL;
            let y = crate::terrain::height(x, z);
            let dhdx =
                (crate::terrain::height(x + e, z) - crate::terrain::height(x - e, z)) / (2.0 * e);
            let dhdz =
                (crate::terrain::height(x, z + e) - crate::terrain::height(x, z - e)) / (2.0 * e);
            let (nx, ny, nz) = (-dhdx, 1.0, -dhdz);
            let nl = (nx * nx + ny * ny + nz * nz).sqrt();
            verts.push(MeshVertex {
                pos: [x, y, z],
                normal: [nx / nl, ny / nl, nz / nl],
                uv: [0.0, 0.0],
            });
        }
    }
    let mut indices = Vec::with_capacity((n * n * 6) as usize);
    for j in 0..n {
        for i in 0..n {
            let a = (j * w + i) as u32;
            let b = (j * w + i + 1) as u32;
            let c = ((j + 1) * w + i) as u32;
            let d = ((j + 1) * w + i + 1) as u32;
            // CCW seen from above → +Y-facing front.
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    TerrainSurface { verts, indices }
}

/// The snapped-patch key for `terrain_surface_mesh`: the surface geometry
/// is identical for any player position within the same cell, so the
/// caller caches it and only regenerates when this key changes.
pub fn surface_snap(px: f32, pz: f32) -> (i32, i32) {
    ((px / SURF_CELL).round() as i32, (pz / SURF_CELL).round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_snap_is_stable_within_a_cell_and_shifts_across_it() {
        // Same cell → same key: the cached surface mesh is reused.
        assert_eq!(surface_snap(0.0, 0.0), surface_snap(SURF_CELL * 0.4, 0.0));
        // Cross a cell → new key: regenerate.
        assert_ne!(surface_snap(0.0, 0.0), surface_snap(0.0, SURF_CELL));
    }

    #[test]
    fn terrain_surface_sits_on_the_heightfield_with_gradient_normals() {
        let s = terrain_surface_mesh(0.0, 0.0);
        assert!(!s.verts.is_empty() && !s.indices.is_empty(), "empty surface");
        assert_eq!(s.indices.len() % 3, 0, "indices are not whole triangles");
        for v in s.verts.iter().step_by(11) {
            let h = crate::terrain::height(v.pos[0], v.pos[2]);
            assert!(
                (v.pos[1] - h).abs() < 1e-2,
                "surface vert off the heightfield at ({},{})",
                v.pos[0],
                v.pos[2]
            );
            let n = v.normal;
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-3, "normal not unit-length ({len})");
        }
        // Open terrain rolls: the surface is not flat.
        let (lo, hi) = s
            .verts
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v.pos[1]), b.max(v.pos[1])));
        assert!(hi - lo > 50.0, "surface flat over open terrain ({:.1})", hi - lo);
    }

    #[test]
    fn terrain_surface_is_flat_with_up_normals_over_a_stamp_pad() {
        // The school pad (pad half 2908); verts well inside the footprint.
        let (ax, az) = (10_800.0f32, 44_400.0f32);
        let s = terrain_surface_mesh(ax, az);
        let pad = crate::terrain::height(ax, az);
        let mut checked = 0;
        for v in &s.verts {
            if (v.pos[0] - ax).abs() < 1500.0 && (v.pos[2] - az).abs() < 1500.0 {
                assert!((v.pos[1] - pad).abs() < 1e-2, "pad surface not flat");
                assert!(v.normal[1] > 0.999, "pad normal not up: {:?}", v.normal);
                checked += 1;
            }
        }
        assert!(checked > 0, "no surface verts sampled inside the pad");
    }
}
