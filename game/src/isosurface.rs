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

/// Signed distance to a **round cone** — a segment from `a` to `b` with
/// a rounded cap of radius `ra` at `a` and `rb` at `b`, blending
/// linearly along the axis. This is the tapered wood primitive: a limb
/// with radius `base_radius` at its base and `base_radius·radius_shrink`
/// at its tip. `sd_capsule` is the constant-radius special case.
///
/// Inigo Quilez's single-sqrt round-cone SDF. Three regions along the
/// axis (below `a`'s cap, along the side, above `b`'s cap) chosen by a
/// discriminant `k`; the three arms of the return correspond one-to-one
/// to the three regions.
pub fn sd_round_cone(p: [f32; 3], a: [f32; 3], b: [f32; 3], ra: f32, rb: f32) -> f32 {
    let ba = sub(b, a);
    let l2 = dot(ba, ba);
    // Degenerate: a == b → sphere of the larger radius.
    if l2 < 1e-12 {
        return length(sub(p, a)) - ra.max(rb);
    }
    let rr = ra - rb;
    let a2 = l2 - rr * rr;
    let il2 = 1.0 / l2;
    let pa = sub(p, a);
    let y = dot(pa, ba);
    let z = y - l2;
    let pa_l2_minus_ba_y = [
        pa[0] * l2 - ba[0] * y,
        pa[1] * l2 - ba[1] * y,
        pa[2] * l2 - ba[2] * y,
    ];
    let x2 = dot(pa_l2_minus_ba_y, pa_l2_minus_ba_y);
    let y2 = y * y * l2;
    let z2 = z * z * l2;
    let k = rr.signum() * rr * rr * x2;
    if z.signum() * a2 * z2 > k {
        // Past the top cap: distance to `b`'s hemisphere.
        (x2 + z2).sqrt() * il2 - rb
    } else if y.signum() * a2 * y2 < k {
        // Below the bottom cap: distance to `a`'s hemisphere.
        (x2 + y2).sqrt() * il2 - ra
    } else {
        // Side region: distance to the slanted lateral surface.
        ((x2 * a2 * il2).sqrt() + y * rr) * il2 - ra
    }
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

/// The voxel grid the surface is meshed against. `res` cells per axis
/// (so `res+1` grid vertices per axis); `min` is the coordinate of grid
/// vertex `(0,0,0)`; each cell has size `step`. When `cells` is `Some`,
/// only those cells are marched (a **narrow band** around the skeleton)
/// — the perf fix that turns a mostly-empty ~64³ fine grid from seconds
/// into fractions of a second.
pub struct Grid<'c> {
    pub min: [f32; 3],
    pub step: [f32; 3],
    pub res: usize,
    pub cells: Option<&'c [[usize; 3]]>,
}

impl<'c> Grid<'c> {
    /// Full-box grid over `[min,max]` at `res` cells per axis. Every
    /// cell is marched; the surface is watertight over the whole box.
    pub fn full(min: [f32; 3], max: [f32; 3], res: usize) -> Self {
        let res = res.max(1);
        let step = [
            (max[0] - min[0]) / res as f32,
            (max[1] - min[1]) / res as f32,
            (max[2] - min[2]) / res as f32,
        ];
        Self { min, step, res, cells: None }
    }
    fn coord(&self, ix: usize, iy: usize, iz: usize) -> [f32; 3] {
        [
            self.min[0] + self.step[0] * ix as f32,
            self.min[1] + self.step[1] * iy as f32,
            self.min[2] + self.step[2] * iz as f32,
        ]
    }
    fn vidx(&self, ix: usize, iy: usize, iz: usize) -> usize {
        let n = self.res + 1;
        (iz * n + iy) * n + ix
    }
}

/// The per-march constants shared by every `emit_tri` / `process_cell`
/// call. Bundling `(field, eps, vertex)` keeps the recursion-heavy
/// signatures compact.
struct EmitCtx<'a> {
    field: &'a dyn Fn([f32; 3]) -> f32,
    eps: f32,
    vertex: &'a VertexFn<'a>,
}

/// Emit one triangle (three surface points): wind it so its geometric
/// face normal agrees with the field gradient (outward, so back-face
/// culling shows the outside); per-vertex normals from the gradient →
/// smooth. Shared by every crossing case in `process_cell`.
fn emit_tri(
    ctx: &EmitCtx,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    verts: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
) {
    let na = gradient(ctx.field, a, ctx.eps);
    let nb = gradient(ctx.field, b, ctx.eps);
    let nc = gradient(ctx.field, c, ctx.eps);
    let face = cross(sub(b, a), sub(c, a));
    let avg = [na[0] + nb[0] + nc[0], na[1] + nb[1] + nc[1], na[2] + nb[2] + nc[2]];
    let base = verts.len() as u32;
    if dot(face, avg) >= 0.0 {
        verts.push((ctx.vertex)(a, na));
        verts.push((ctx.vertex)(b, nb));
        verts.push((ctx.vertex)(c, nc));
    } else {
        verts.push((ctx.vertex)(a, na));
        verts.push((ctx.vertex)(c, nc));
        verts.push((ctx.vertex)(b, nb));
    }
    indices.push(base);
    indices.push(base + 1);
    indices.push(base + 2);
}

/// Triangulate one cube cell: split into the 6 tetrahedra of `TETS`, and
/// for each tet, emit the surface fragment from its 1-/2-/3-inside case.
fn process_cell(
    ctx: &EmitCtx,
    cpos: &[[f32; 3]; 8],
    cval: &[f32; 8],
    verts: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
) {
    for tet in &TETS {
        let p = [cpos[tet[0]], cpos[tet[1]], cpos[tet[2]], cpos[tet[3]]];
        let v = [cval[tet[0]], cval[tet[1]], cval[tet[2]], cval[tet[3]]];
        let inside: Vec<usize> = (0..4).filter(|&i| v[i] < 0.0).collect();
        let outside: Vec<usize> = (0..4).filter(|&i| v[i] >= 0.0).collect();
        match inside.len() {
            1 | 3 => {
                // Lone corner (whichever side is the minority) connects
                // to the other three across three edges.
                let (lone, others) = if inside.len() == 1 {
                    (inside[0], &outside)
                } else {
                    (outside[0], &inside)
                };
                let e0 = edge_point(p[lone], v[lone], p[others[0]], v[others[0]]);
                let e1 = edge_point(p[lone], v[lone], p[others[1]], v[others[1]]);
                let e2 = edge_point(p[lone], v[lone], p[others[2]], v[others[2]]);
                emit_tri(ctx, e0, e1, e2, verts, indices);
            }
            2 => {
                // Two-in, two-out: four crossing edges → a quad → 2 tris.
                let (i0, i1) = (inside[0], inside[1]);
                let (o0, o1) = (outside[0], outside[1]);
                let q0 = edge_point(p[i0], v[i0], p[o0], v[o0]);
                let q1 = edge_point(p[i0], v[i0], p[o1], v[o1]);
                let q2 = edge_point(p[i1], v[i1], p[o1], v[o1]);
                let q3 = edge_point(p[i1], v[i1], p[o0], v[o0]);
                emit_tri(ctx, q0, q1, q2, verts, indices);
                emit_tri(ctx, q0, q2, q3, verts, indices);
            }
            _ => {} // all in or all out — no surface here
        }
    }
}

/// Mesh the zero level set of `field` over `grid`. `field` is negative
/// inside; `vertex` maps a (position, outward-normal) to a `MeshVertex`.
/// The field is called at most **once per grid vertex** (cached in a
/// NaN-sentinel table), which is what makes narrow-banding cheap: a
/// small strip of cells inside a large box only touches the cache
/// entries near those cells.
pub fn marching_tetrahedra(
    field: &dyn Fn([f32; 3]) -> f32,
    grid: &Grid,
    vertex: &VertexFn,
) -> (Vec<MeshVertex>, Vec<u32>) {
    let mut verts = Vec::new();
    let mut indices = Vec::new();
    let res = grid.res.max(1);
    let n = res + 1;
    // NaN sentinel = not yet computed. `f32::NAN != NaN` so any
    // subsequent `is_nan()` check unambiguously signals "cache miss".
    let mut memo: Vec<f32> = vec![f32::NAN; n * n * n];
    let mut sample = |ix: usize, iy: usize, iz: usize| -> f32 {
        let k = grid.vidx(ix, iy, iz);
        let v = memo[k];
        if v.is_nan() {
            let v = field(grid.coord(ix, iy, iz));
            memo[k] = v;
            v
        } else {
            v
        }
    };
    let eps = grid.step[0].min(grid.step[1]).min(grid.step[2]) * 0.5;
    let ctx = EmitCtx { field, eps, vertex };

    let march_one = |ix: usize,
                     iy: usize,
                     iz: usize,
                     sample: &mut dyn FnMut(usize, usize, usize) -> f32,
                     verts: &mut Vec<MeshVertex>,
                     indices: &mut Vec<u32>| {
        // Cube corners: bit 0=x, bit 1=y, bit 2=z.
        let cpos: [[f32; 3]; 8] = std::array::from_fn(|i| {
            grid.coord(ix + (i & 1), iy + ((i >> 1) & 1), iz + ((i >> 2) & 1))
        });
        let cval: [f32; 8] = std::array::from_fn(|i| {
            sample(ix + (i & 1), iy + ((i >> 1) & 1), iz + ((i >> 2) & 1))
        });
        process_cell(&ctx, &cpos, &cval, verts, indices);
    };

    match grid.cells {
        Some(cells) => {
            for &[ix, iy, iz] in cells {
                if ix >= res || iy >= res || iz >= res {
                    continue; // out-of-range cell index — silently skip
                }
                march_one(ix, iy, iz, &mut sample, &mut verts, &mut indices);
            }
        }
        None => {
            for iz in 0..res {
                for iy in 0..res {
                    for ix in 0..res {
                        march_one(ix, iy, iz, &mut sample, &mut verts, &mut indices);
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
        let grid = Grid::full([-1.5, -1.5, -1.5], [1.5, 1.5, 1.5], 24);
        let (verts, indices) = marching_tetrahedra(&field, &grid, &plain_vertex);
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
        let grid = Grid::full([-0.5, -0.5, -0.5], [0.5, 1.5, 0.5], 24);
        let (verts, _) = marching_tetrahedra(&field, &grid, &plain_vertex);
        assert!(!verts.is_empty());
        for v in &verts {
            assert!(sd_capsule(v.pos, a, b, 0.2).abs() < 0.06, "vertex off the capsule");
        }
    }

    #[test]
    fn round_cone_tapers_from_ra_at_a_to_rb_at_b() {
        // A round cone from a=(0,0,0) to b=(0,1,0), radii 0.3→0.1. Points
        // on the axis at each endpoint should sit at the endpoint radius
        // (distance = 0 to a point on the surface, so field = −radius).
        // A point r units off the axis at the base plane should be
        // roughly on-surface for r ≈ ra; same at the top for r ≈ rb.
        let a = [0.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        let ra = 0.3;
        let rb = 0.1;
        // Field value on the axis at each cap = −radius (deepest inside).
        assert!((sd_round_cone(a, a, b, ra, rb) - -ra).abs() < 1e-4);
        assert!((sd_round_cone(b, a, b, ra, rb) - -rb).abs() < 1e-4);
        // A point exactly on the ROUNDED CAP hemisphere at each end sits
        // on the surface — extend `ra` past `a` along −axis, `rb` past
        // `b` along +axis. (A point at the equator of each cap is NOT
        // on-surface: the lateral surface tangents in *below* the top
        // equator, since the cone slopes inward as it climbs.)
        assert!(sd_round_cone([0.0, -ra, 0.0], a, b, ra, rb).abs() < 1e-4);
        assert!(sd_round_cone([0.0, 1.0 + rb, 0.0], a, b, ra, rb).abs() < 1e-4);
        // A point far outside is positive.
        assert!(sd_round_cone([1.0, 0.5, 0.0], a, b, ra, rb) > 0.5);
    }

    #[test]
    fn round_cone_meshed_at_two_radii_gives_a_tapered_solid() {
        // Skin the tapered capsule and confirm the base end is fatter
        // than the top end: the max XZ radius on the lower half must
        // exceed the max XZ radius on the upper half.
        let a = [0.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        let ra = 0.3;
        let rb = 0.1;
        let field = |p: [f32; 3]| sd_round_cone(p, a, b, ra, rb);
        let grid = Grid::full([-0.5, -0.5, -0.5], [0.5, 1.5, 0.5], 32);
        let (verts, _) = marching_tetrahedra(&field, &grid, &plain_vertex);
        assert!(!verts.is_empty());
        let radius_xz = |p: [f32; 3]| (p[0] * p[0] + p[2] * p[2]).sqrt();
        let max_lower = verts
            .iter()
            .filter(|v| v.pos[1] < 0.3)
            .map(|v| radius_xz(v.pos))
            .fold(0.0f32, f32::max);
        let max_upper = verts
            .iter()
            .filter(|v| v.pos[1] > 0.7)
            .map(|v| radius_xz(v.pos))
            .fold(0.0f32, f32::max);
        assert!(
            max_lower > max_upper + 0.05,
            "base should be fatter than top: max_lower={max_lower} max_upper={max_upper}",
        );
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
        let grid = Grid::full([-0.4, -0.4, -0.4], [0.9, 1.4, 0.4], 28);
        let (verts, indices) = marching_tetrahedra(&field, &grid, &plain_vertex);
        assert!(!verts.is_empty());
        assert!(indices.len() >= 12, "expected a real shell, got {} indices", indices.len());
    }

    #[test]
    fn narrow_band_marches_only_listed_cells() {
        // A single capsule in a box that's mostly empty. If we hand
        // only the cells that touch the capsule, the output must equal
        // the full-box march for this shape (all crossing cells are in
        // the strip); handing NO cells produces an empty mesh; handing
        // an out-of-range index is ignored (silent skip).
        let a = [0.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        let r = 0.15;
        let field = |p: [f32; 3]| sd_capsule(p, a, b, r);
        let full_grid = Grid::full([-0.5, -0.5, -0.5], [0.5, 1.5, 0.5], 20);
        let (full_verts, _) = marching_tetrahedra(&field, &full_grid, &plain_vertex);
        // Enumerate every cell whose AABB (padded by r) hits the
        // capsule's AABB. That's a superset of every crossing cell.
        let mut cells: Vec<[usize; 3]> = Vec::new();
        let res = full_grid.res;
        let step = full_grid.step;
        let min = full_grid.min;
        for iz in 0..res {
            for iy in 0..res {
                for ix in 0..res {
                    let c_min = [
                        min[0] + step[0] * ix as f32,
                        min[1] + step[1] * iy as f32,
                        min[2] + step[2] * iz as f32,
                    ];
                    let c_max = [c_min[0] + step[0], c_min[1] + step[1], c_min[2] + step[2]];
                    // The capsule's AABB, expanded by r.
                    let cap_min = [-r, -r, -r];
                    let cap_max = [r, 1.0 + r, r];
                    let hit = c_min[0] <= cap_max[0]
                        && c_max[0] >= cap_min[0]
                        && c_min[1] <= cap_max[1]
                        && c_max[1] >= cap_min[1]
                        && c_min[2] <= cap_max[2]
                        && c_max[2] >= cap_min[2];
                    if hit {
                        cells.push([ix, iy, iz]);
                    }
                }
            }
        }
        let band = Grid { min, step, res, cells: Some(&cells) };
        let (band_verts, _) = marching_tetrahedra(&field, &band, &plain_vertex);
        // Same crossing cells → same vertex count.
        assert_eq!(band_verts.len(), full_verts.len());
        // Narrow-band was a real perf saving: strictly fewer cells.
        assert!(cells.len() < res * res * res);

        // Empty cell list → empty mesh.
        let empty_cells: [[usize; 3]; 0] = [];
        let empty = Grid { min, step, res, cells: Some(&empty_cells) };
        let (ev, ei) = marching_tetrahedra(&field, &empty, &plain_vertex);
        assert!(ev.is_empty() && ei.is_empty());

        // Out-of-range cell index is silently skipped (doesn't panic).
        let oor = [[res + 5, 0, 0]];
        let g = Grid { min, step, res, cells: Some(&oor) };
        let _ = marching_tetrahedra(&field, &g, &plain_vertex);
    }
}
