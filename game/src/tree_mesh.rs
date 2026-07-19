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

/// A double-sided unit **leaf card**: a flat quad in the local XZ plane
/// (normal +Y), 1×1 centred on the origin, UV spanning [0,1]². A leaf is
/// a flat blade, not a ball — clusters of oriented cards read as foliage
/// that catches light per-face. Both windings are emitted so the card is
/// visible from either side (a single-sided quad vanishes when it faces
/// away under back-face culling). The instance's `axis` rotates +Y → the
/// leaf's facing direction; instance scale sets width (x) × length (z).
pub fn leaf_quad_mesh() -> (Vec<MeshVertex>, Vec<u32>) {
    let n = [0.0, 1.0, 0.0];
    let v = |x: f32, z: f32, u: f32, w: f32| MeshVertex {
        pos: [x, 0.0, z],
        normal: n,
        uv: [u, w],
    };
    let verts = vec![
        v(-0.5, -0.5, 0.0, 0.0),
        v(0.5, -0.5, 1.0, 0.0),
        v(0.5, 0.5, 1.0, 1.0),
        v(-0.5, 0.5, 0.0, 1.0),
    ];
    // Front face, then the same quad reverse-wound — double-sided.
    let indices = vec![0, 1, 2, 0, 2, 3, 0, 2, 1, 0, 3, 2];
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

/// One limb of the branch skeleton — a tapered cone from `base` along
/// `axis` for `length`, `base_radius` thick at the base. Everything is
/// in UNIT tree space (trunk base at the origin, trunk ≈ 1 tall) so the
/// consumer scales by the tree's height, exactly like `canopy_stations`
/// scales by `canopy_radius`. `is_tip` marks a terminal limb — a leaf
/// cluster anchors at its tip (`base + axis*length`), so foliage follows
/// the branches instead of hanging in a cloud around the trunk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BranchSegment {
    pub base: [f32; 3],
    pub axis: [f32; 3],
    pub length: f32,
    pub base_radius: f32,
    pub is_tip: bool,
    /// A terminal twig that died — greyed limb, no leaf cluster. A
    /// scatter of these on a living tree reads as real (not every branch
    /// leafs out); on the fungal species they're where witch's snot
    /// clings. Only ever set on tips (`is_tip`).
    pub is_dead: bool,
}

impl BranchSegment {
    /// World-space (unit) tip of the limb.
    pub fn tip(&self) -> [f32; 3] {
        [
            self.base[0] + self.axis[0] * self.length,
            self.base[1] + self.axis[1] * self.length,
            self.base[2] + self.axis[2] * self.length,
        ]
    }
}

/// Deterministic recursive branch skeleton for one tree. `seed` varies
/// the structure per tree (peers pass the same seed → identical tree);
/// the whole thing lives in unit space so the caller scales by height.
/// Primary limbs leave the upper trunk, each recursing into child
/// limbs to `BRANCH_MAX_DEPTH` — branches, and branches' branches —
/// with terminal tips flagged as leaf anchors. Pure: same seed → the
/// same Vec, byte for byte.
/// Per-species tree shape + appearance. One struct fed to the generator
/// and the emit turns the same code into a pine, an oak, or a birch.
/// `(lo, hi)` pairs are sampled per limb from the tree's seed. Geometry
/// fields are unit-space; appearance fields (× height, colours) are read
/// by `snapshot_to_mesh_instances`.
#[derive(Clone, Copy, Debug)]
pub struct TreeSpecies {
    pub primaries: (u32, u32),
    pub base_y: (f32, f32),
    /// Radians off vertical for a primary limb — the biggest silhouette
    /// axis: pine wide/horizontal, birch narrow/upright, oak moderate.
    pub primary_spread: (f32, f32),
    pub primary_len: (f32, f32),
    /// trunk-limb → branch → … → tip. ≥1; deeper = gnarlier.
    pub max_depth: u32,
    pub child_spread: (f32, f32),
    pub len_shrink: f32,
    /// Each child limb's radius ÷ its parent's. < 1, so radius shrinks
    /// monotonically from the trunk (root) out to the tips — a branch can
    /// never be thicker than the wood it grows from.
    pub radius_shrink: f32,
    /// Radius of the ROOT limb (the trunk) in unit tree space. It is the
    /// depth-0 limb of the same recursion every branch comes from — there
    /// is no separate "trunk" concept. Its length is `base_y.1` (the bole
    /// rises to the highest primary); primaries branch off it at
    /// `base_y.0..base_y.1`, each `trunk_radius × radius_shrink` thick.
    pub trunk_radius: f32,
    pub trunk_color: [f32; 3],
    pub branch_color: [f32; 3],
    pub leaves_per_tip: u32,
    pub cluster_radius_ratio: f32,
    pub leaf_element_ratio: f32,
    /// Leaf-card length ÷ width. ~1.3 = broad blade (oak), higher = a
    /// slimmer/needle-ish leaf (pine).
    pub leaf_aspect: f32,
    pub leaf_green: [f32; 3],
    /// Ceiling of the per-leaf autumn-age ramp: 0 = evergreen (pine),
    /// higher = more/warmer turn (oak → red, birch → yellow).
    pub autumn: f32,
    /// If the species fruits, the fruit colour. `Some` → some trees of
    /// this species grow a scatter of small round bodies (apples in the
    /// crown, or witch's snot on dead limbs — see `fruit_on_dead_limbs`);
    /// `None` → nothing grows. Which trees bear is a per-tree roll off the
    /// tree seed, so a stand is a mix of fruiting and bare trees.
    pub fruit_color: Option<[f32; 3]>,
    /// Where the fruit bodies grow. `false` → apples: hang below LIVE
    /// tips, most trees, ~a third of tips. `true` → witch's snot: clings
    /// at DEAD tips (`BranchSegment::is_dead`), nearly every tree, most
    /// dead tips (the fungal species — "find a mushroom" made visual).
    pub fruit_on_dead_limbs: bool,
    /// Per-TREE probability this species carries any dead twigs. Only
    /// SOME trees do (it's a probability < 1, rolled once off the tree
    /// seed), and only trees of a species that can — a sapling sets this
    /// to 0.0 so young trees are never gnarled with deadwood. Within a
    /// bearing tree, each tip then dies with `DEAD_TIP_CHANCE`.
    pub dead_limb_odds: f32,
    /// Size multiplier for AUTHORED (CDDA-placed) trees of this species,
    /// applied on top of the near-uniform authored height. 1.0 = the
    /// base tended height; an orchard apple sits a touch above 1 so it
    /// reads bigger than a yard sapling without becoming wild old-growth.
    /// Procedural forest trees ignore this (they size off `tree_at_cell`).
    pub authored_scale: f32,
    /// Per-segment trunk bend, in radians. 0 = perfectly straight (pine,
    /// birch — ramrod species). Higher = the trunk kinks over its
    /// height, modelling apical-dominance loss / phototropism where a
    /// side branch effectively takes over. Applied as `TRUNK_SEGMENTS`
    /// stacked segments each tilted by this angle from the previous in
    /// a single per-tree direction; the tune HUD's `trunk_curve_mult`
    /// scales it at runtime.
    pub trunk_curvature: f32,
}

/// Conifer: tall narrow column, many short near-horizontal whorls
/// shrinking toward a point, shallow recursion, dark dense evergreen.
pub static PINE: TreeSpecies = TreeSpecies {
    primaries: (7, 9),
    base_y: (0.15, 0.85),
    primary_spread: (1.0, 1.3),
    primary_len: (0.12, 0.22),
    max_depth: 1,
    child_spread: (0.3, 0.6),
    len_shrink: 0.6,
    radius_shrink: 0.6,
    trunk_radius: 0.02,
    trunk_color: [0.22, 0.15, 0.10],
    branch_color: [0.25, 0.17, 0.11],
    leaves_per_tip: 18,
    cluster_radius_ratio: 0.045,
    leaf_element_ratio: 0.02,
    leaf_aspect: 4.0,
    leaf_green: [0.08, 0.42, 0.22],
    autumn: 0.0,
    fruit_color: None,
    authored_scale: 1.0,
    fruit_on_dead_limbs: false,
    dead_limb_odds: 0.4,
    trunk_curvature: 0.0,
};

/// Broadleaf spreader: thick trunk, few long forking limbs, deep
/// recursion (gnarled), broad round crown that turns in autumn.
pub static OAK: TreeSpecies = TreeSpecies {
    primaries: (3, 4),
    base_y: (0.30, 0.55),
    primary_spread: (0.5, 0.9),
    primary_len: (0.24, 0.34),
    max_depth: 3,
    child_spread: (0.5, 0.95),
    len_shrink: 0.62,
    radius_shrink: 0.62,
    trunk_radius: 0.035,
    trunk_color: [0.30, 0.20, 0.11],
    branch_color: [0.34, 0.23, 0.13],
    leaves_per_tip: 20,
    cluster_radius_ratio: 0.08,
    leaf_element_ratio: 0.028,
    leaf_aspect: 1.3,
    leaf_green: [0.13, 0.70, 0.32],
    autumn: 0.5,
    fruit_color: None,
    authored_scale: 1.0,
    fruit_on_dead_limbs: false,
    dead_limb_odds: 0.45,
    trunk_curvature: 0.08,
};

/// Slender upright: thin pale trunk, branches that point up, airy
/// canopy, bright-yellow autumn.
pub static BIRCH: TreeSpecies = TreeSpecies {
    trunk_curvature: 0.03,
    primaries: (3, 5),
    base_y: (0.35, 0.75),
    primary_spread: (0.2, 0.5),
    primary_len: (0.18, 0.28),
    max_depth: 2,
    child_spread: (0.3, 0.6),
    len_shrink: 0.6,
    radius_shrink: 0.6,
    trunk_radius: 0.018,
    trunk_color: [0.72, 0.72, 0.68],
    branch_color: [0.55, 0.55, 0.50],
    leaves_per_tip: 14,
    cluster_radius_ratio: 0.06,
    leaf_element_ratio: 0.024,
    leaf_aspect: 1.8,
    leaf_green: [0.35, 0.72, 0.28],
    autumn: 0.3,
    fruit_color: None,
    authored_scale: 1.0,
    fruit_on_dead_limbs: false,
    dead_limb_odds: 0.40,
};

/// Broad, low, drooping umbrella: many near-horizontal limbs, dense
/// trailing foliage, soft green with a faint yellow turn — a willow.
pub static WILLOW: TreeSpecies = TreeSpecies {
    trunk_curvature: 0.15,
    primaries: (5, 7),
    base_y: (0.28, 0.50),
    primary_spread: (1.1, 1.4),
    primary_len: (0.24, 0.34),
    max_depth: 2,
    child_spread: (0.4, 0.9),
    len_shrink: 0.68,
    radius_shrink: 0.6,
    trunk_radius: 0.028,
    trunk_color: [0.28, 0.20, 0.12],
    branch_color: [0.32, 0.24, 0.14],
    leaves_per_tip: 16,
    cluster_radius_ratio: 0.06,
    leaf_element_ratio: 0.022,
    leaf_aspect: 2.5,
    leaf_green: [0.30, 0.68, 0.32],
    autumn: 0.25,
    fruit_color: None,
    authored_scale: 1.0,
    fruit_on_dead_limbs: false,
    dead_limb_odds: 0.40,
};

/// Deterministic species pick for a tree seed — a mixed woodland: oak
/// common, pine and birch less so. Peers pass the same seed → same
/// species.
pub fn species_for(seed: u32) -> &'static TreeSpecies {
    match (seed >> 13) % 20 {
        0..=5 => &OAK, // common
        6..=8 => &MAPLE,
        9..=11 => &PINE,
        12..=14 => &BIRCH,
        15..=16 => &WILLOW,
        17 => &DEAD, // a rare snag
        _ => &OAK,
    }
}

/// Broadleaf like the oak but a fiery turn — yellow-green running to
/// orange/red. A maple.
pub static MAPLE: TreeSpecies = TreeSpecies {
    trunk_curvature: 0.06,
    primaries: (3, 5),
    base_y: (0.30, 0.55),
    primary_spread: (0.5, 0.9),
    primary_len: (0.24, 0.34),
    max_depth: 3,
    child_spread: (0.5, 0.95),
    len_shrink: 0.62,
    radius_shrink: 0.62,
    trunk_radius: 0.034,
    trunk_color: [0.34, 0.22, 0.12],
    branch_color: [0.38, 0.25, 0.14],
    leaves_per_tip: 20,
    cluster_radius_ratio: 0.075,
    leaf_element_ratio: 0.028,
    leaf_aspect: 1.3,
    leaf_green: [0.62, 0.58, 0.16],
    autumn: 1.0,
    fruit_color: None,
    authored_scale: 1.0,
    fruit_on_dead_limbs: false,
    dead_limb_odds: 0.45,
};

/// Alien fungal growth — greyish trunk, muted purple foliage, evergreen.
pub static FUNGAL: TreeSpecies = TreeSpecies {
    trunk_curvature: 0.12,
    primaries: (4, 6),
    base_y: (0.25, 0.55),
    primary_spread: (0.6, 1.0),
    primary_len: (0.18, 0.28),
    max_depth: 2,
    child_spread: (0.5, 0.9),
    len_shrink: 0.6,
    radius_shrink: 0.6,
    trunk_radius: 0.03,
    trunk_color: [0.45, 0.40, 0.48],
    branch_color: [0.50, 0.44, 0.54],
    leaves_per_tip: 16,
    cluster_radius_ratio: 0.07,
    leaf_element_ratio: 0.028,
    leaf_aspect: 1.4,
    leaf_green: [0.55, 0.30, 0.62],
    autumn: 0.0,
    fruit_color: Some([0.80, 0.85, 0.32]),
    authored_scale: 1.0,
    fruit_on_dead_limbs: true,
    dead_limb_odds: 0.70,
};

/// A dead snag — bare branch skeleton, no foliage (`leaves_per_tip = 0`),
/// weathered grey-brown. A stark silhouette among the leafy trees.
pub static DEAD: TreeSpecies = TreeSpecies {
    trunk_curvature: 0.10,
    primaries: (3, 5),
    base_y: (0.30, 0.60),
    primary_spread: (0.6, 1.1),
    primary_len: (0.22, 0.32),
    max_depth: 3,
    child_spread: (0.6, 1.1),
    len_shrink: 0.6,
    radius_shrink: 0.6,
    trunk_radius: 0.028,
    trunk_color: [0.30, 0.26, 0.22],
    branch_color: [0.34, 0.30, 0.25],
    leaves_per_tip: 0,
    cluster_radius_ratio: 0.0,
    leaf_element_ratio: 0.0,
    leaf_aspect: 1.0,
    leaf_green: [0.0, 0.0, 0.0],
    autumn: 0.0,
    fruit_color: None,
    authored_scale: 1.0,
    fruit_on_dead_limbs: false,
    dead_limb_odds: 0.0,
};

/// Small round fruit tree — short trunk, dense rounded crown, broad
/// leaves. The orchard species (CDDA `t_tree_apple` and friends).
pub static APPLE: TreeSpecies = TreeSpecies {
    trunk_curvature: 0.10,
    primaries: (4, 6),
    base_y: (0.22, 0.42),
    primary_spread: (0.7, 1.1),
    primary_len: (0.16, 0.24),
    max_depth: 2,
    child_spread: (0.5, 0.9),
    len_shrink: 0.62,
    radius_shrink: 0.62,
    trunk_radius: 0.03,
    trunk_color: [0.32, 0.22, 0.13],
    branch_color: [0.36, 0.25, 0.15],
    leaves_per_tip: 18,
    cluster_radius_ratio: 0.09,
    leaf_element_ratio: 0.026,
    leaf_aspect: 1.2,
    leaf_green: [0.16, 0.62, 0.26],
    autumn: 0.2,
    fruit_color: Some([0.80, 0.14, 0.11]),
    authored_scale: 1.3,
    fruit_on_dead_limbs: false,
    dead_limb_odds: 0.30,
};

/// Map the importer's framework-free `TreeKind` (from CDDA `t_tree_*`)
/// to the game's rendered species. This is the game-side half of the
/// bridge — the importer says "apple here", we decide what an apple
/// looks like.
pub fn species_for_kind(kind: cdda::TreeKind) -> &'static TreeSpecies {
    match kind {
        cdda::TreeKind::Apple => &APPLE,
        cdda::TreeKind::Pine => &PINE,
        cdda::TreeKind::Oak => &OAK,
        cdda::TreeKind::Birch => &BIRCH,
        cdda::TreeKind::Willow => &WILLOW,
        cdda::TreeKind::Maple => &MAPLE,
        cdda::TreeKind::Fungal => &FUNGAL,
        cdda::TreeKind::Dead => &DEAD,
        // A CDDA stump's original species is lost — render a cut oak bole.
        // The `stump` flag (set at spawn) is what makes it a stump; the
        // species only supplies the bark.
        cdda::TreeKind::Stump => &OAK,
        cdda::TreeKind::Generic => &OAK,
    }
}

/// Deterministic per-position species — a stable hash of the tile the
/// tree stands on. The seam the CDDA bridge plugs into: procedural
/// trees pick here, authored (CDDA `t_tree_*`) trees override with
/// `species_for_kind`.
pub fn species_for_pos(x: f32, z: f32) -> &'static TreeSpecies {
    let mut h: u32 = 2_166_136_261;
    for b in x.to_bits().to_le_bytes().iter().chain(z.to_bits().to_le_bytes().iter()) {
        h ^= *b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    species_for(h)
}

fn rangef(rng: &mut u32, (lo, hi): (f32, f32)) -> f32 {
    lo + randf(rng) * (hi - lo)
}

// Deterministic PRNG (splitmix32) + minimal vec3 helpers. No external
// rng, no floats-from-time — the skeleton is a pure function of `seed`
// so every peer draws the identical tree.
fn splitmix32(state: &mut u32) -> u32 {
    *state = state.wrapping_add(0x9E37_79B9);
    let mut z = *state;
    z = (z ^ (z >> 16)).wrapping_mul(0x21f0_aaad);
    z = (z ^ (z >> 15)).wrapping_mul(0x735a_2d97);
    z ^ (z >> 15)
}
fn randf(state: &mut u32) -> f32 {
    (splitmix32(state) >> 8) as f32 / (1u32 << 24) as f32
}
fn v_add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn v_scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn v_norm(a: [f32; 3]) -> [f32; 3] {
    let m = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(f32::EPSILON);
    [a[0] / m, a[1] / m, a[2] / m]
}
fn v_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Orthonormal basis (u, v) spanning the plane ⟂ `dir`.
fn perp_basis(dir: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let refv = if dir[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = v_norm(v_cross(refv, dir));
    let v = v_cross(dir, u);
    (u, v)
}

/// Grow one limb from `base` along `dir` for `len` as a single tapered
/// cone segment, then recurse into `depth` more child limbs fanned
/// around the tip at golden-angle azimuths. At depth 0 the segment is a
/// terminal — its tip is a leaf anchor.
/// Chance a tip on a deadwood-bearing tree is a bare dead twig.
const DEAD_TIP_CHANCE: f32 = 0.18;

// A recursive limb generator: the accumulators (out, rng) and the
// per-limb geometry (base, dir, len, radius, depth) plus the species
// context genuinely are distinct arguments; bundling them into a struct
// just to satisfy the arg-count heuristic would obscure the recursion.
#[allow(clippy::too_many_arguments)]
fn grow_limb(
    out: &mut Vec<BranchSegment>,
    base: [f32; 3],
    dir: [f32; 3],
    len: f32,
    radius: f32,
    depth: u32,
    sp: &TreeSpecies,
    tree_dead: bool,
    rng: &mut u32,
) {
    // A terminal twig dies with a small probability, but ONLY on a tree
    // that bears deadwood at all (`tree_dead`, decided once per tree) —
    // so deadwood is a trait of some trees, not a uniform speckle on
    // every tree. Deterministic (drawn from the tree's rng, peers agree).
    // Only tips can die; interior limbs always carry on.
    let is_tip = depth == 0;
    let is_dead = is_tip && tree_dead && (splitmix32(rng) % 1000) < (DEAD_TIP_CHANCE * 1000.0) as u32;
    out.push(BranchSegment {
        base,
        axis: dir,
        length: len,
        base_radius: radius,
        is_tip,
        is_dead,
    });
    if depth == 0 {
        return;
    }
    let tip = v_add(base, v_scale(dir, len));
    let children = 2 + (splitmix32(rng) % 2) as usize; // 2 or 3
    let (u, v) = perp_basis(dir);
    let azim0 = randf(rng) * std::f32::consts::TAU;
    for c in 0..children {
        let spread = rangef(rng, sp.child_spread); // off the parent
        let phi = azim0 + (c as f32) * GOLDEN_ANGLE_RAD;
        let radial = v_add(v_scale(u, phi.cos()), v_scale(v, phi.sin()));
        let child_dir = v_norm(v_add(v_scale(dir, spread.cos()), v_scale(radial, spread.sin())));
        grow_limb(
            out,
            tip,
            child_dir,
            len * sp.len_shrink,
            radius * sp.radius_shrink,
            depth - 1,
            sp,
            tree_dead,
            rng,
        );
    }
}

/// Number of stacked segments in the curved trunk. Each bends slightly
/// from the previous; 4 gives room for a real S-curve without being so
/// coarse the "bends" read as elbows.
pub const TRUNK_SEGMENTS: u32 = 4;

/// One trunk segment's book-keeping used to attach primaries: end
/// position, this segment's axis, its length, its base radius, its top
/// radius. Grouped once so `point_along_trunk` doesn't need a 5-tuple.
type TrunkTop = ([f32; 3], [f32; 3], f32, f32, f32);

pub fn tree_branches(seed: u32, sp: &TreeSpecies) -> Vec<BranchSegment> {
    // Spread the seed through the state and force it odd/non-zero so
    // even seed 0 gives a non-degenerate tree.
    let mut rng = seed.wrapping_mul(2_654_435_761).wrapping_add(0x9E37_79B9) | 1;
    let mut out = Vec::new();
    // Decide ONCE whether this whole tree carries deadwood — only some
    // trees of a species do, and a sapling species (odds 0) never does.
    let tree_dead = randf(&mut rng) < sp.dead_limb_odds;

    // Trunk: N stacked segments, each rotated slightly from the previous
    // in a single per-tree direction (chosen from the RNG). Species with
    // `trunk_curvature == 0` collapse to a straight vertical bole. The
    // runtime tune multiplier scales it further so the HUD's
    // `trunk_curve_mult` acts on all species uniformly.
    let curve_mult = crate::tune::get().trunk_curve_mult;
    let curve = sp.trunk_curvature * curve_mult;
    let bend_phi = randf(&mut rng) * std::f32::consts::TAU;
    let bend_dir = [bend_phi.cos(), 0.0, bend_phi.sin()];
    let seg_len = sp.base_y.1 / TRUNK_SEGMENTS as f32;
    let mut trunk_axis = [0.0f32, 1.0, 0.0];
    let mut trunk_base = [0.0f32, 0.0, 0.0];
    // Per-segment stored data for primary attachment: (end position,
    // this segment's axis, its length, its base_radius, its top_radius).
    let mut trunk_tops: Vec<TrunkTop> = Vec::with_capacity(TRUNK_SEGMENTS as usize);
    for i in 0..TRUNK_SEGMENTS {
        // Geometric radius shrink across segments so segment i+1's base
        // radius equals segment i's top radius — no beading at the
        // joints. Segment 0 starts at trunk_radius; segment N-1 ends at
        // trunk_radius × radius_shrink^N, matching where thin outer
        // branches would live.
        let base_r = sp.trunk_radius * sp.radius_shrink.powi(i as i32);
        let top_r = base_r * sp.radius_shrink;
        // Per-segment tilt: bend by `curve` radians toward `bend_dir`.
        // Small-angle tilt+renormalize approximates a rotation of
        // `axis` around the horizontal perpendicular by `curve`.
        let tilt = curve;
        let mut new_axis = [
            trunk_axis[0] + bend_dir[0] * tilt,
            trunk_axis[1],
            trunk_axis[2] + bend_dir[2] * tilt,
        ];
        let l = (new_axis[0] * new_axis[0]
            + new_axis[1] * new_axis[1]
            + new_axis[2] * new_axis[2])
            .sqrt()
            .max(1e-6);
        new_axis = [new_axis[0] / l, new_axis[1] / l, new_axis[2] / l];
        out.push(BranchSegment {
            base: trunk_base,
            axis: new_axis,
            length: seg_len,
            base_radius: base_r,
            is_tip: false,
            is_dead: false,
        });
        let end = [
            trunk_base[0] + new_axis[0] * seg_len,
            trunk_base[1] + new_axis[1] * seg_len,
            trunk_base[2] + new_axis[2] * seg_len,
        ];
        trunk_tops.push((end, new_axis, seg_len, base_r, top_r));
        trunk_base = end;
        trunk_axis = new_axis;
    }

    // Primaries branch OFF the curved trunk at fractions between
    // base_y.0/base_y.1 (lowest attachment) and 1.0 (top). The along-
    // trunk coordinate walks the stacked segments to find the world
    // position — otherwise the primaries would sit on a phantom
    // straight y-axis and float off the curved bole.
    let lo_frac = sp.base_y.0 / sp.base_y.1.max(1e-6);
    let span = (sp.primaries.1 - sp.primaries.0 + 1).max(1);
    let primaries = sp.primaries.0 + splitmix32(&mut rng) % span;
    let azim0 = randf(&mut rng) * std::f32::consts::TAU;
    for c in 0..primaries {
        let frac = c as f32 / primaries.max(1) as f32;
        let t = lo_frac + (1.0 - lo_frac) * frac;
        let (base, _local_axis, local_r) = point_along_trunk(&trunk_tops, t, sp.base_y.1);
        let spread = rangef(&mut rng, sp.primary_spread);
        let phi = azim0 + (c as f32) * GOLDEN_ANGLE_RAD;
        let radial = [phi.cos(), 0.0, phi.sin()];
        let dir = v_norm(v_add([0.0, spread.cos(), 0.0], v_scale(radial, spread.sin())));
        let len = rangef(&mut rng, sp.primary_len) * (1.0 - 0.4 * frac);
        // Primary radius derives from the LOCAL trunk radius at the
        // attach point, not the fat root trunk_radius — so a primary
        // branching high on the tree (where the trunk is thin) is
        // itself thin, not ballooning out.
        let primary_radius = local_r * sp.radius_shrink;
        grow_limb(&mut out, base, dir, len, primary_radius, sp.max_depth, sp, tree_dead, &mut rng);
    }
    out
}

/// Sample a point along the curved trunk at along-trunk fraction t.
/// `tops` is one entry per trunk segment: (end position, axis, length,
/// base_radius, top_radius). `total_len` = sum of segment lengths ≈
/// base_y.1. Returns (position, axis at that point, local trunk radius
/// linearly interpolated between the segment's base and top radii).
fn point_along_trunk(tops: &[TrunkTop], t: f32, total_len: f32) -> ([f32; 3], [f32; 3], f32) {
    let target = t * total_len;
    let mut acc = 0.0f32;
    let mut base = [0.0f32, 0.0, 0.0];
    for &(end, axis, len, base_r, top_r) in tops {
        if acc + len >= target {
            let f = ((target - acc) / len).clamp(0.0, 1.0);
            return (
                [
                    base[0] + axis[0] * len * f,
                    base[1] + axis[1] * len * f,
                    base[2] + axis[2] * len * f,
                ],
                axis,
                base_r * (1.0 - f) + top_r * f,
            );
        }
        acc += len;
        base = end;
    }
    // t past the top: last segment's tip.
    let &(end, axis, _len, _base_r, top_r) = tops.last().expect("trunk has at least one segment");
    (end, axis, top_r)
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
        for tri in indices.as_chunks::<3>().0 {
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
    fn leaf_quad_is_a_flat_double_sided_card() {
        let (verts, indices) = leaf_quad_mesh();
        assert_eq!(verts.len(), 4, "a quad has 4 corners");
        assert_eq!(indices.len(), 12, "two triangles per face × two faces");
        for v in &verts {
            assert!(v.pos[1].abs() < 1e-6, "leaf card must be flat (y=0): {:?}", v.pos);
            assert!(
                (0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1]),
                "uv out of [0,1]: {:?}",
                v.uv
            );
        }
        for &i in &indices {
            assert!((i as usize) < verts.len(), "index {i} out of range");
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

    // ---- branch skeleton contract ----

    const SPECIES: [&TreeSpecies; 8] =
        [&PINE, &OAK, &BIRCH, &WILLOW, &APPLE, &MAPLE, &FUNGAL, &DEAD];

    #[test]
    fn branches_are_deterministic_in_the_seed() {
        // Peers pass the same seed + species and must resolve the same tree.
        assert_eq!(tree_branches(42, &OAK), tree_branches(42, &OAK));
    }

    #[test]
    fn different_seeds_give_different_trees() {
        // The whole point: variety. Two seeds must not clone.
        assert_ne!(tree_branches(1, &OAK), tree_branches(2, &OAK));
    }

    #[test]
    fn recursion_produces_leaf_tips() {
        // "branches, and branches' branches" — terminal tips exist and
        // there are several of them (not a lone trunk).
        let tips = tree_branches(7, &OAK).iter().filter(|p| p.is_tip).count();
        assert!(tips >= 4, "expected several leaf tips, got {tips}");
    }

    #[test]
    fn the_trunk_is_segment_zero_and_no_limb_out_fattens_it() {
        // The trunk is the ROOT of the recursion (segment 0), the tree's
        // thickest wood; every other limb derives its radius from it via
        // radius_shrink (< 1), so NO branch can ever be thicker than the
        // trunk it grows from. This is guaranteed by construction, not by
        // hand-tuned per-species radii that used to disagree.
        for sp in SPECIES {
            let segs = tree_branches(7, sp);
            let root_r = segs[0].base_radius;
            assert_eq!(root_r, sp.trunk_radius, "segment 0 must be the trunk");
            assert!(!segs[0].is_tip, "the trunk is not a leaf tip");
            // The first TRUNK_SEGMENTS segments are all trunk stacks —
            // they share `trunk_radius` on purpose (the bole doesn't
            // taper along its own height; radius shrinkage starts at
            // the primaries). Only check downstream limbs.
            for s in &segs[TRUNK_SEGMENTS as usize..] {
                assert!(
                    s.base_radius < root_r,
                    "a limb (r={}) out-fattened the trunk (r={root_r})",
                    s.base_radius
                );
            }
        }
    }

    #[test]
    fn deadwood_is_a_per_tree_trait_bounded_by_species_odds() {
        // A sapling-like species (odds 0.0 — DEAD stands in) never grows
        // dead twigs: young/opted-out trees aren't gnarled with deadwood.
        for seed in 0..200u32 {
            assert!(
                tree_branches(seed, &DEAD).iter().all(|s| !s.is_dead),
                "an odds-0 species must have zero deadwood"
            );
        }
        // OAK (odds 0.45) carries deadwood on SOME trees, not all — it's a
        // per-tree trait, not a uniform speckle on every tree.
        let (mut with, mut without) = (0, 0);
        for seed in 0..200u32 {
            if tree_branches(seed, &OAK).iter().any(|s| s.is_dead) {
                with += 1;
            } else {
                without += 1;
            }
        }
        assert!(with > 0 && without > 0, "deadwood must vary tree-to-tree: {with} with, {without} without");
        // And only tips ever die — interior limbs always carry on.
        for seed in 0..80u32 {
            for s in tree_branches(seed, &OAK) {
                if s.is_dead {
                    assert!(s.is_tip, "only tips may be dead");
                }
            }
        }
    }

    #[test]
    fn branches_taper_with_depth() {
        // A terminal limb is thinner than the thickest (trunk-side) one.
        let segs = tree_branches(7, &OAK);
        let max_r = segs.iter().map(|s| s.base_radius).fold(0.0_f32, f32::max);
        let tip_r = segs
            .iter()
            .filter(|s| s.is_tip)
            .map(|s| s.base_radius)
            .fold(f32::INFINITY, f32::min);
        assert!(
            tip_r < max_r,
            "terminal limbs ({tip_r}) should be thinner than the base ({max_r})"
        );
    }

    #[test]
    fn every_species_stays_in_unit_space_with_tips_in_the_crown() {
        // Every species must keep its limbs in unit space (so the caller
        // scales by height) and reach tips up into the crown.
        for sp in SPECIES {
            for seed in [1u32, 7, 42, 1000] {
                let segs = tree_branches(seed, sp);
                assert!(!segs.is_empty(), "no segments for a species");
                for s in &segs {
                    for pt in [s.base, s.tip()] {
                        assert!(pt[1] >= -EPS, "endpoint below ground: {pt:?}");
                        assert!(pt[1] <= 1.6, "endpoint too tall: {pt:?}");
                        let rxz = (pt[0] * pt[0] + pt[2] * pt[2]).sqrt();
                        assert!(rxz <= 1.0, "endpoint too wide: {pt:?}");
                    }
                }
                let max_tip_y = segs
                    .iter()
                    .filter(|s| s.is_tip)
                    .map(|s| s.tip()[1])
                    .fold(0.0_f32, f32::max);
                assert!(max_tip_y > 0.5, "tips should reach the crown, got {max_tip_y}");
            }
        }
    }

    #[test]
    fn branch_recursion_terminates_for_every_species() {
        for sp in SPECIES {
            assert!(tree_branches(7, sp).len() < 4000);
        }
    }

    #[test]
    fn species_pick_is_deterministic_and_varied() {
        // Same seed → same species; across seeds we see more than one.
        assert!(std::ptr::eq(species_for(12345), species_for(12345)));
        let mut seen = std::collections::HashSet::new();
        for seed in 0..200u32 {
            seen.insert(species_for(seed.wrapping_mul(2_654_435_761)).primaries);
        }
        assert!(seen.len() >= 2, "species pick collapsed to one variety");
    }

    #[test]
    fn species_shape_the_tree_differently() {
        // Same seed, different species → different trees (pine's many
        // shallow whorls vs birch's few upright limbs can't coincide).
        assert_ne!(tree_branches(7, &PINE), tree_branches(7, &BIRCH));
    }
}
