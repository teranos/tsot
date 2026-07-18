//! One continuous woody surface — the tree's trunk, branches, and buttress
//! roots as ONE isosurface, not a pile of stuck-together cones. Every
//! limb becomes a tapered round-cone SDF; five root cones flare from the
//! bole into the ground; the smooth-min (`smin`) of all of them is the
//! field, and marching tetrahedra pulls its zero level set out as a
//! triangle mesh. Forks blend, the base flares — automatically — because
//! adjacent primitives share one distance function.
//!
//! Unit tree space: y=0 is ground, the bole rises to y ≈ `sp.base_y.1`.
//! The caller scales by height and offsets to world. Pure fn of
//! `(seed, species)` → cache elsewhere; nothing here is stateful.
//!
//! Cost: dominated by the narrow-banded march (a small strip of cells
//! around the skeleton, ~0.3s/tree release). The field itself is a
//! straight `smin`-fold over ~dozens of round-cones per grid vertex,
//! cached by `marching_tetrahedra` so each grid vertex is evaluated at
//! most once regardless of how many tets touch it.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use crate::isosurface::{Grid, marching_tetrahedra, sd_round_cone, smin};
use crate::tree_mesh::{BranchSegment, MeshVertex, TreeSpecies, tree_branches};

/// One canonical wood mesh per species — `Rc`-shared, generated on first
/// call, kept forever. Every tree of that species instances THIS mesh
/// (scaled + positioned per instance), so wood cost across the world is
/// O(species) — a fixed small number — instead of O(unique trees).
pub type WoodMesh = Rc<(Vec<MeshVertex>, Vec<u32>)>;

/// Canonical seed used for the per-species wood generation. Every tree
/// of the species shares this skeleton; per-tree variation (girth,
/// moss, deadwood, autumn tint) rides on the instance, not the mesh.
const CANONICAL_SEED: u32 = 0;

thread_local! {
    /// key = species-ptr-as-usize. Species is `&'static`, so ptr equality
    /// is species equality. At most one entry per species (~8) —
    /// bounded, no eviction, no cap.
    static SPECIES_WOOD: RefCell<HashMap<usize, WoodMesh>> =
        RefCell::new(HashMap::new());
}

/// The canonical wood mesh for `sp`. Generated on first call from
/// `tree_surface(CANONICAL_SEED, sp)`, cached forever thread-locally.
/// Every tree of the species instances this mesh — that's what makes
/// the browser path viable at all.
///
/// Bounded budget: 8 species × ~1 MB each ≈ 8 MB retained, ever. No
/// per-tree cache, no eviction, no memoizer.
pub fn species_wood_mesh(sp: &'static TreeSpecies) -> WoodMesh {
    let key = sp as *const TreeSpecies as usize;
    if let Some(hit) = SPECIES_WOOD.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let (verts, indices) = tree_surface(CANONICAL_SEED, sp);
    let mesh: WoodMesh = Rc::new((verts, indices));
    SPECIES_WOOD.with(|c| {
        c.borrow_mut().insert(key, mesh.clone());
    });
    mesh
}

/// Per §3b: 5 buttress roots.
const ROOT_COUNT: usize = 5;
/// Resolution floor + ceiling. Verts per tree scale ~res² under
/// narrow-band. Middle band: fine enough to read as continuous wood at
/// the trunk + main forks, coarse enough that per-tree cache footprint
/// stays proportional.
const RES_MIN: usize = 16;
const RES_MAX: usize = 32;

/// A round-cone in unit tree space. The trunk, every branch, and every
/// root is one of these — the whole tree is a `smin` over a `Vec<Cone>`.
#[derive(Clone, Copy)]
struct Cone {
    a: [f32; 3],
    b: [f32; 3],
    ra: f32,
    rb: f32,
}

/// The one continuous woody surface for the tree. `seed` selects the
/// per-tree structure (same seed → identical mesh); `sp` picks species.
/// Deterministic byte-for-byte in the inputs.
///
/// Output is world-oriented but UNIT-SIZED — caller scales by tree
/// height and adds tree position. `MeshVertex.uv` carries a cylindrical
/// bark UV so the existing bark fragment shader lights the surface with
/// the same procedural furrows the instanced trunks use.
pub fn tree_surface(seed: u32, sp: &TreeSpecies) -> (Vec<MeshVertex>, Vec<u32>) {
    let segs = tree_branches(seed, sp);
    let cones = collect_cones(seed, sp, &segs);

    let (min, max) = aabb(&cones, sp.trunk_radius * 2.0);
    let span = (max[0] - min[0]).max(max[1] - min[1]).max(max[2] - min[2]);
    // Resolution grows with the tree's own span vs its trunk thickness —
    // a slender pine and a broad oak both get roughly the same "cells
    // across the bole" of ~0.9 (RES lets slenderness fit fewer voxels).
    let res = ((span / (sp.trunk_radius * 0.9)).ceil() as usize).clamp(RES_MIN, RES_MAX);
    let step = [span / res as f32; 3];
    let voxel = step[0];
    // Radius floor + blend: below one voxel, marching tets can't resolve
    // a limb (it slips between grid lines). Floor every limb radius to a
    // hair over one voxel so the finest twigs stay continuous — chunky
    // is the trade for "not vanishing".
    let rfloor = voxel * 1.1;
    // Smin blend radius: the fillet where two touching limbs melt into
    // one surface. Just wider than a voxel so a fork always spans one
    // cell of blending.
    let blend = voxel * 1.2;

    let cones_field: Vec<Cone> = cones
        .iter()
        .map(|c| Cone {
            a: c.a,
            b: c.b,
            ra: c.ra.max(rfloor),
            rb: c.rb.max(rfloor),
        })
        .collect();

    // Field: smin over every round-cone. `f32::INFINITY` is the identity
    // — smin(∞, d, k) = d (the exponential falloff has hh=0), so the
    // first fold step just adopts the first cone's distance.
    let field = |p: [f32; 3]| {
        let mut d = f32::INFINITY;
        for c in &cones_field {
            let cd = sd_round_cone(p, c.a, c.b, c.ra, c.rb);
            d = smin(d, cd, blend);
        }
        d
    };

    let cells = narrow_band(&cones_field, min, step, res, blend);
    let cells_vec: Vec<[usize; 3]> = cells.into_iter().collect();
    let grid = Grid { min, step, res, cells: Some(&cells_vec) };

    let vertex = |p: [f32; 3], n: [f32; 3]| -> MeshVertex {
        // Cylindrical bark UV about the trunk axis (unit-space y).
        // atan2 wraps at ±π, and the trunk's procedural bark is periodic
        // in `u` — so the seam falls on identical texture, invisible.
        let u = p[2].atan2(p[0]) / std::f32::consts::TAU + 0.5;
        let v = p[1] * 12.0;
        MeshVertex { pos: p, normal: n, uv: [u, v] }
    };

    marching_tetrahedra(&field, &grid, &vertex)
}

fn collect_cones(seed: u32, sp: &TreeSpecies, segs: &[BranchSegment]) -> Vec<Cone> {
    let mut cones = Vec::with_capacity(segs.len() + ROOT_COUNT);
    // Every skeleton limb is one round-cone, tapering from base_radius
    // at the base to base_radius·radius_shrink at the tip. That preserves
    // the "no child fatter than its parent" invariant baked into
    // `tree_branches`.
    for s in segs {
        let tip = s.tip();
        let rb = s.base_radius * sp.radius_shrink;
        cones.push(Cone { a: s.base, b: tip, ra: s.base_radius, rb });
    }
    // Five buttress roots, angled outward from the top of the ankle
    // (y = trunk_radius·1.5, so the flare rises INTO the bole rather
    // than being a stump on top of it) down and out into the ground.
    // Their phase spreads on seed — no two trees flare in the same
    // direction, and the same seed always flares the same way.
    let ankle_y = sp.trunk_radius * 1.5;
    let reach = sp.trunk_radius * 7.0;
    let depth = sp.trunk_radius * 3.5;
    let ra = sp.trunk_radius * 1.15;
    let rb = sp.trunk_radius * 0.35;
    let seed_phase = (seed as f32) * crate::tree_mesh::GOLDEN_ANGLE_RAD;
    for i in 0..ROOT_COUNT {
        let theta = seed_phase + (i as f32 / ROOT_COUNT as f32) * std::f32::consts::TAU;
        let a = [0.0, ankle_y, 0.0];
        let b = [theta.cos() * reach, -depth, theta.sin() * reach];
        cones.push(Cone { a, b, ra, rb });
    }
    cones
}

fn aabb(cones: &[Cone], pad: f32) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for c in cones {
        let r = c.ra.max(c.rb);
        for k in 0..3 {
            lo[k] = lo[k].min(c.a[k].min(c.b[k]) - r);
            hi[k] = hi[k].max(c.a[k].max(c.b[k]) + r);
        }
    }
    for k in 0..3 {
        lo[k] -= pad;
        hi[k] += pad;
    }
    (lo, hi)
}

/// The set of grid cells whose AABB overlaps any cone's AABB (expanded
/// by radius + blend + one voxel). Ordered — a `BTreeSet` keeps iteration
/// deterministic, so the same `(seed, species)` yields the same index
/// stream out of `marching_tetrahedra`.
fn narrow_band(
    cones: &[Cone],
    grid_min: [f32; 3],
    step: [f32; 3],
    res: usize,
    blend: f32,
) -> BTreeSet<[usize; 3]> {
    let mut cells: BTreeSet<[usize; 3]> = BTreeSet::new();
    let voxel = step[0];
    for c in cones {
        let r = c.ra.max(c.rb) + blend + voxel;
        let lo = [
            c.a[0].min(c.b[0]) - r,
            c.a[1].min(c.b[1]) - r,
            c.a[2].min(c.b[2]) - r,
        ];
        let hi = [
            c.a[0].max(c.b[0]) + r,
            c.a[1].max(c.b[1]) + r,
            c.a[2].max(c.b[2]) + r,
        ];
        let ix_lo = cell_ix(lo[0], grid_min[0], step[0], res);
        let iy_lo = cell_ix(lo[1], grid_min[1], step[1], res);
        let iz_lo = cell_ix(lo[2], grid_min[2], step[2], res);
        let ix_hi = cell_ix(hi[0], grid_min[0], step[0], res);
        let iy_hi = cell_ix(hi[1], grid_min[1], step[1], res);
        let iz_hi = cell_ix(hi[2], grid_min[2], step[2], res);
        for iz in iz_lo..=iz_hi {
            for iy in iy_lo..=iy_hi {
                for ix in ix_lo..=ix_hi {
                    cells.insert([ix, iy, iz]);
                }
            }
        }
    }
    cells
}

fn cell_ix(p: f32, grid_min: f32, step: f32, res: usize) -> usize {
    let idx = ((p - grid_min) / step).floor() as i32;
    idx.clamp(0, res as i32 - 1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_mesh::{OAK, PINE};

    #[test]
    fn species_wood_mesh_is_one_shared_rc_per_species() {
        // Every call for the same species must hand back the same `Rc`
        // — that's what makes the design bounded (8 species → 8 meshes,
        // ever). Different species → distinct meshes.
        let a = species_wood_mesh(&OAK);
        let b = species_wood_mesh(&OAK);
        assert!(Rc::ptr_eq(&a, &b), "second call regenerated");
        let c = species_wood_mesh(&PINE);
        assert!(!Rc::ptr_eq(&a, &c));
    }

    #[test]
    fn species_wood_mesh_matches_tree_surface_output_byte_for_byte() {
        // The species cache MUST return exactly what `tree_surface`
        // would with the canonical seed. Any drift here ships wrong
        // geometry silently.
        let cached = species_wood_mesh(&OAK);
        let (fresh_v, fresh_i) = tree_surface(CANONICAL_SEED, &OAK);
        assert_eq!(cached.0.len(), fresh_v.len());
        assert_eq!(cached.1, fresh_i);
        for (a, b) in cached.0.iter().zip(fresh_v.iter()) {
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.normal, b.normal);
            assert_eq!(a.uv, b.uv);
        }
    }

    fn depth(sp: &TreeSpecies) -> f32 {
        sp.trunk_radius * 3.5
    }

    #[test]
    fn tree_surface_produces_a_bounded_mesh_with_a_trunk_on_the_axis() {
        // Skin an oak. Every vertex must lie inside a generous bounding
        // volume (the tree's own extents plus slack), and the trunk must
        // actually appear on the y-axis — i.e. some vertex near y=0.2
        // sits within a couple trunk-radii of the axis. If either fails,
        // we're either not meshing what we think we are, or the trunk
        // isn't in the union.
        let (verts, indices) = tree_surface(0, &OAK);
        assert!(!verts.is_empty(), "no geometry emitted");
        assert_eq!(indices.len() % 3, 0);

        let d = depth(&OAK);
        for v in &verts {
            assert!(v.pos[1] >= -d - 0.5, "vertex below root reach: y={}", v.pos[1]);
            assert!(v.pos[1] <= OAK.base_y.1 + 1.0, "vertex above bole top: y={}", v.pos[1]);
            let r = (v.pos[0] * v.pos[0] + v.pos[2] * v.pos[2]).sqrt();
            // Horizontal reach: primary limbs extend outward; a very
            // loose horizontal bound keeps this a "no runaway geometry"
            // check, not a species-shape check.
            assert!(r < 5.0, "vertex outside horizontal bound: r={r}");
        }
        // Trunk presence: at some height in the bole, there must be a
        // vertex near the trunk-radius circle around the axis.
        let y_probe = OAK.base_y.1 * 0.4;
        let near_bole = verts.iter().any(|v| {
            (v.pos[1] - y_probe).abs() < 0.05
                && ((v.pos[0] * v.pos[0] + v.pos[2] * v.pos[2]).sqrt()
                    < OAK.trunk_radius * 4.0)
        });
        assert!(near_bole, "no trunk-adjacent vertex at y ≈ {y_probe}");
    }

    #[test]
    fn tree_surface_flares_roots_below_ground() {
        // The roots reach `-trunk_radius·3.5` below y=0. There must be
        // real surface below y=0 (the buttress flare) — otherwise the
        // isosurface stopped at the bole and the roots aren't in.
        let (verts, _) = tree_surface(7, &OAK);
        let below = verts.iter().filter(|v| v.pos[1] < -0.001).count();
        assert!(
            below > 0,
            "no vertices below ground — roots missing from the union",
        );
    }

    #[test]
    fn tree_surface_is_deterministic_in_seed_and_species() {
        // Same (seed, species) → byte-identical output. The narrow-band
        // set is a BTreeSet so iteration order is deterministic; the
        // field composition is a straight fold; MeshVertex is Copy of
        // f32s. Any nondeterminism here would break replay + peer sync.
        let (v1, i1) = tree_surface(42, &OAK);
        let (v2, i2) = tree_surface(42, &OAK);
        assert_eq!(i1, i2);
        assert_eq!(v1.len(), v2.len());
        for (a, b) in v1.iter().zip(v2.iter()) {
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.normal, b.normal);
            assert_eq!(a.uv, b.uv);
        }
    }
}
