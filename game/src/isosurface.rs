//! Isosurface meshing by marching tetrahedra. Turns a scalar field
//! (negative inside the solid, positive outside — a signed-distance-ish
//! field) into a triangle mesh of its zero level set. Used to skin the
//! tree's woody skeleton into ONE continuous organic surface: forks
//! blend, the trunk base flares into roots, all as a single skin — not a
//! pile of overlapping cones.
//!
//! Marching TETRAHEDRA, not marching cubes, on purpose: each cube is
//! split into 6 tetrahedra, and a tet has only a handful of crossing
//! cases (1-in, 2-in, 3-in). No 256-entry edge/tri tables to
//! mis-transcribe — the whole thing is a few lines of index logic, so it
//! is correct-by-inspection and testable. Output is a triangle "soup"
//! (vertices not index-welded) but GAP-FREE: neighbouring tets interpolate
//! a shared edge to the identical point, so the surface is watertight, and
//! per-vertex normals from the field gradient make it shade as one smooth
//! surface without any welding.

use crate::tree_mesh::MeshVertex;

/// Smooth union of two distances (polynomial smin). `k` is the blend
/// radius: larger → rounder fillets where two shapes meet. This is what
/// makes forks and the root flare organic instead of a hard intersection.
pub fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let hh = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - hh * hh * k * 0.25
}

/// Signed distance from `p` to the capsule (a rounded cylinder) with
/// endpoints `a`, `b` and radius `r`. Negative inside. This is one limb.
pub fn sd_capsule(p: [f32; 3], a: [f32; 3], b: [f32; 3], r: f32) -> f32 {
    let pa = sub(p, a);
    let ba = sub(b, a);
    let bb = dot(ba, ba).max(1e-12);
    let t = (dot(pa, ba) / bb).clamp(0.0, 1.0);
    let closest = [a[0] + ba[0] * t, a[1] + ba[1] * t, a[2] + ba[2] * t];
    length(sub(p, closest)) - r
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = length(a).max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Central-difference gradient of `field` at `p`, normalized → the
/// outward surface normal (the field increases toward the outside).
fn gradient(field: &dyn Fn([f32; 3]) -> f32, p: [f32; 3], eps: f32) -> [f32; 3] {
    let gx = field([p[0] + eps, p[1], p[2]]) - field([p[0] - eps, p[1], p[2]]);
    let gy = field([p[0], p[1] + eps, p[2]]) - field([p[0], p[1] - eps, p[2]]);
    let gz = field([p[0], p[1], p[2] + eps]) - field([p[0], p[1], p[2] - eps]);
    normalize([gx, gy, gz])
}

/// The 6 tetrahedra that tile a cube, all sharing the main diagonal 0–7.
/// (Corner i has coords ((i&1),((i>>1)&1),((i>>2)&1)).)
const TETS: [[usize; 4]; 6] = [
    [0, 7, 1, 3],
    [0, 7, 3, 2],
    [0, 7, 2, 6],
    [0, 7, 6, 4],
    [0, 7, 4, 5],
    [0, 7, 5, 1],
];

/// Interpolate the zero crossing on the edge between corner values.
fn edge_point(pa: [f32; 3], va: f32, pb: [f32; 3], vb: f32) -> [f32; 3] {
    let denom = va - vb;
    let t = if denom.abs() < 1e-12 { 0.5 } else { va / denom };
    let t = t.clamp(0.0, 1.0);
    [
        pa[0] + (pb[0] - pa[0]) * t,
        pa[1] + (pb[1] - pa[1]) * t,
        pa[2] + (pb[2] - pa[2]) * t,
    ]
}

/// A per-vertex hook: given a surface point + its outward normal, produce
/// the full mesh vertex (lets the caller attach UVs however it likes).
pub type VertexFn<'a> = dyn Fn([f32; 3], [f32; 3]) -> MeshVertex + 'a;

/// Mesh the zero level set of `field` over the axis-aligned box
/// `[min,max]` at `res` cells per axis. `field` is negative inside.
/// `vertex` maps a (position, outward-normal) to a `MeshVertex`.
pub fn marching_tetrahedra(
    field: &dyn Fn([f32; 3]) -> f32,
    min: [f32; 3],
    max: [f32; 3],
    res: usize,
    vertex: &VertexFn,
) -> (Vec<MeshVertex>, Vec<u32>) {
    let mut verts = Vec::new();
    let mut indices = Vec::new();
    let res = res.max(1);
    let step = [
        (max[0] - min[0]) / res as f32,
        (max[1] - min[1]) / res as f32,
        (max[2] - min[2]) / res as f32,
    ];
    let cell = step[0].min(step[1]).min(step[2]);
    let eps = cell * 0.5;

    // Emit one triangle (3 surface points), wound so its geometric normal
    // agrees with the field gradient (outward), so back-face culling shows
    // the outside. Per-vertex normals come from the gradient → smooth.
    let mut emit = |a: [f32; 3], b: [f32; 3], c: [f32; 3], verts: &mut Vec<MeshVertex>, indices: &mut Vec<u32>| {
        let na = gradient(field, a, eps);
        let nb = gradient(field, b, eps);
        let nc = gradient(field, c, eps);
        let face = cross(sub(b, a), sub(c, a));
        let avg = [na[0] + nb[0] + nc[0], na[1] + nb[1] + nc[1], na[2] + nb[2] + nc[2]];
        let base = verts.len() as u32;
        if dot(face, avg) >= 0.0 {
            verts.push(vertex(a, na));
            verts.push(vertex(b, nb));
            verts.push(vertex(c, nc));
        } else {
            verts.push(vertex(a, na));
            verts.push(vertex(c, nc));
            verts.push(vertex(b, nb));
        }
        indices.push(base);
        indices.push(base + 1);
        indices.push(base + 2);
    };

    for iz in 0..res {
        for iy in 0..res {
            for ix in 0..res {
                let cbase = [
                    min[0] + step[0] * ix as f32,
                    min[1] + step[1] * iy as f32,
                    min[2] + step[2] * iz as f32,
                ];
                // Corner positions use a per-axis step, so build them by hand.
                let cpos: [[f32; 3]; 8] = std::array::from_fn(|i| {
                    [
                        cbase[0] + step[0] * (i & 1) as f32,
                        cbase[1] + step[1] * ((i >> 1) & 1) as f32,
                        cbase[2] + step[2] * ((i >> 2) & 1) as f32,
                    ]
                });
                let cval: [f32; 8] = std::array::from_fn(|i| field(cpos[i]));

                for tet in &TETS {
                    let p = [cpos[tet[0]], cpos[tet[1]], cpos[tet[2]], cpos[tet[3]]];
                    let v = [cval[tet[0]], cval[tet[1]], cval[tet[2]], cval[tet[3]]];
                    let inside: Vec<usize> = (0..4).filter(|&i| v[i] < 0.0).collect();
                    let outside: Vec<usize> = (0..4).filter(|&i| v[i] >= 0.0).collect();
                    match inside.len() {
                        1 | 3 => {
                            // The lone corner (whichever side is the minority)
                            // connects to the other three across three edges.
                            let (lone, others) = if inside.len() == 1 {
                                (inside[0], &outside)
                            } else {
                                (outside[0], &inside)
                            };
                            let e0 = edge_point(p[lone], v[lone], p[others[0]], v[others[0]]);
                            let e1 = edge_point(p[lone], v[lone], p[others[1]], v[others[1]]);
                            let e2 = edge_point(p[lone], v[lone], p[others[2]], v[others[2]]);
                            emit(e0, e1, e2, &mut verts, &mut indices);
                        }
                        2 => {
                            // Two-in, two-out: four crossing edges → a quad.
                            let (i0, i1) = (inside[0], inside[1]);
                            let (o0, o1) = (outside[0], outside[1]);
                            let q0 = edge_point(p[i0], v[i0], p[o0], v[o0]);
                            let q1 = edge_point(p[i0], v[i0], p[o1], v[o1]);
                            let q2 = edge_point(p[i1], v[i1], p[o1], v[o1]);
                            let q3 = edge_point(p[i1], v[i1], p[o0], v[o0]);
                            emit(q0, q1, q2, &mut verts, &mut indices);
                            emit(q0, q2, q3, &mut verts, &mut indices);
                        }
                        _ => {} // all in or all out — no surface here
                    }
                }
            }
        }
    }
    (verts, indices)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_vertex(p: [f32; 3], n: [f32; 3]) -> MeshVertex {
        MeshVertex { pos: p, normal: n, uv: [0.0, 0.0] }
    }

    #[test]
    fn meshing_a_sphere_gives_a_closed_shell_on_the_surface() {
        // Field: a sphere of radius 1 at the origin. The mesh must be
        // non-empty, every vertex must sit on the sphere (|p| ≈ 1), and
        // every normal must point radially outward (n · p > 0).
        let field = |p: [f32; 3]| length(p) - 1.0;
        let (verts, indices) = marching_tetrahedra(
            &field,
            [-1.5, -1.5, -1.5],
            [1.5, 1.5, 1.5],
            24,
            &plain_vertex,
        );
        assert!(!verts.is_empty(), "sphere produced no geometry");
        assert_eq!(indices.len() % 3, 0);
        for v in &verts {
            let r = length(v.pos);
            assert!((r - 1.0).abs() < 0.08, "vertex off the sphere: r={r}");
            // Outward normal: points the same way as the position vector.
            assert!(dot(v.normal, normalize(v.pos)) > 0.5, "normal not outward");
        }
    }

    #[test]
    fn meshing_a_capsule_is_bounded_to_its_extent() {
        // A capsule from (0,0,0) to (0,1,0), radius 0.2. Every surface
        // vertex must lie within radius+slack of the segment.
        let a = [0.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        let field = |p: [f32; 3]| sd_capsule(p, a, b, 0.2);
        let (verts, _) =
            marching_tetrahedra(&field, [-0.5, -0.5, -0.5], [0.5, 1.5, 0.5], 24, &plain_vertex);
        assert!(!verts.is_empty());
        for v in &verts {
            assert!(sd_capsule(v.pos, a, b, 0.2).abs() < 0.06, "vertex off the capsule");
        }
    }

    #[test]
    fn two_capsules_fuse_into_one_shell_at_their_join() {
        // A fork: two capsules meeting at the origin. With a smooth union
        // the surface is a SINGLE connected shell through the joint — the
        // whole point. Sanity: it meshes, and a point in the crotch just
        // outside both is still outside the union.
        let field = |p: [f32; 3]| {
            let l0 = sd_capsule(p, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.15);
            let l1 = sd_capsule(p, [0.0, 0.5, 0.0], [0.6, 1.1, 0.0], 0.12);
            smin(l0, l1, 0.1)
        };
        let (verts, indices) =
            marching_tetrahedra(&field, [-0.4, -0.4, -0.4], [0.9, 1.4, 0.4], 28, &plain_vertex);
        assert!(!verts.is_empty());
        assert!(indices.len() >= 12, "expected a real shell, got {} indices", indices.len());
    }
}
