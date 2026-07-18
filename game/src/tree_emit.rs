//! game/src/tree_emit — the tree mesh emit subsystem. Turns the scene
//! snapshot's tree list into the two flat mesh-pipeline instance lists
//! (trunks + canopy elements): tapered trunk/branch cones plus leaf
//! cards, fruit, moss, nests, and splinters. Deterministic per tree so
//! every peer draws the same skeleton and autumn tint.

use crate::scene::{MeshInstance, SceneSnapshot};
use crate::tree_mesh::MeshVertex;
use crate::tree_surface::tree_surface_cached;

/// Deterministic per-tree seed from world position — same tile → same
/// seed on every peer, so the branch skeleton is identical everywhere.
fn tree_seed(x: f32, z: f32) -> u32 {
    let mut h: u32 = 2_166_136_261;
    for b in x.to_bits().to_le_bytes().iter().chain(z.to_bits().to_le_bytes().iter()) {
        h ^= *b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    h
}

/// The tree-emit outputs feeding the mesh pipeline.
///
/// - `trunks` + `canopy_elements` — the instanced-cone path: every trunk
///   instance draws the shared tapered-cone geometry, every canopy element
///   draws the shared unit icosahedron.
/// - `wood_verts` + `wood_indices` — the continuous-wood path
///   (CONTINUOUS_WOOD.md): every tree's woody skeleton merged into ONE
///   world-space vertex+index buffer, drawn once as a single identity
///   `MeshInstance`. Filled by `snapshot_to_mesh_instances_with_wood`;
///   empty from `snapshot_to_mesh_instances` (the legacy browser path).
///
/// Native `render.rs` draws wood + canopy; the browser path still draws
/// trunks + canopy until step 4 flips it.
pub struct MeshTreeInstances {
    pub trunks: Vec<MeshInstance>,
    pub canopy_elements: Vec<MeshInstance>,
    pub wood_verts: Vec<MeshVertex>,
    pub wood_indices: Vec<u32>,
}

/// Build the mesh-pipeline instance lists from the scene snapshot.
/// Deterministic in the snapshot's tree list. Emits `N` trunk cone
/// instances plus, per tree, a recursive branch skeleton
/// (`tree_branches`) rendered as brown limb spheres with green leaf
/// clusters at each terminal tip — all packed into `canopy_elements`
/// with per-instance colour. The render loop packs trunks + canopy
/// elements into one instance buffer (trunks first) and issues two
/// `render_mesh` calls with the right `first_instance` offset.
/// Hash a per-leaf index to a uniform [0,1) — deterministic, so a leaf's
/// autumn tint is identical on every peer.
fn leaf_hash01(seed: u32, idx: u32) -> f32 {
    let mut h = seed ^ idx.wrapping_mul(0x9E37_79B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x21f0_aaad);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Map a leaf's `age` ∈ [0,1] through the autumn ramp: the species' green
/// → yellow → orange → red → brown. age 0 keeps the leaf green.
fn autumn_ramp(green: [f32; 3], age: f32) -> [f32; 3] {
    const YELLOW: [f32; 3] = [0.85, 0.75, 0.15];
    const ORANGE: [f32; 3] = [0.85, 0.45, 0.10];
    const RED: [f32; 3] = [0.62, 0.14, 0.10];
    const BROWN: [f32; 3] = [0.36, 0.24, 0.12];
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    if age <= 0.4 {
        mix(green, YELLOW, age / 0.4)
    } else if age <= 0.6 {
        mix(YELLOW, ORANGE, (age - 0.4) / 0.2)
    } else if age <= 0.8 {
        mix(ORANGE, RED, (age - 0.6) / 0.2)
    } else {
        mix(RED, BROWN, (age - 0.8) / 0.2)
    }
}

pub fn snapshot_to_mesh_instances(snap: &SceneSnapshot) -> MeshTreeInstances {
    use crate::tree_mesh::{GOLDEN_ANGLE_RAD, tree_branches};
    // axis = [dir.xyz, sway]. UP is the vertical, rigid orientation (sway
    // 0) shared by the trunk, stump, and nest.
    const UP: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    // `trunks` draws the shared unit cone (trunk + every branch segment);
    // `canopy_elements` draws the shared leaf card (oriented per leaf).
    let mut trunks = Vec::with_capacity(snap.trees.len() * 48);
    let mut canopy_elements = Vec::with_capacity(snap.trees.len() * 256);
    for (t, h, sp, stump) in &snap.trees {
        let h = *h;
        // Species is carried on the tree (data, not a render-time hash) —
        // procedural trees filled it from the tile, authored CDDA trees
        // will name their own. `seed` still drives per-leaf autumn tint.
        let sp: &crate::tree_mesh::TreeSpecies = sp;
        let seed = tree_seed(t.x, t.z);
        // Per-tree girth: some trees are far stouter than others (old
        // growth vs young), so a stand isn't a row of identical trunks.
        // Squared so most trees are ordinary and a few are notably fat.
        let g = leaf_hash01(seed, 0x61_2711);
        // One girth factor scales ALL the tree's wood uniformly (the trunk
        // is the root of the recursion, so the branches derive their radius
        // from it and inherit the fattening) — a stout tree is stout all
        // the way out, not a fat bole with mismatched thin limbs.
        let girth = 0.75 + 1.6 * g * g; // ~0.75 .. 2.35, few fat
        // A stump is the short remainder of a felled tree of THIS species:
        // a stout low bole in the species' bark, capped by a pale cut face
        // (the mesh bark furrows read as rings on the flat top). No crown,
        // no branches — then we're done with this tree.
        if *stump {
            let sh = h * 0.11; // knee-high remainder
            let sr = h * sp.trunk_radius * girth * 1.4; // stout
            trunks.push(MeshInstance {
                pos: [t.x, 0.0, t.z],
                color: sp.trunk_color,
                scale: [sr, sh, sr],
                axis: UP,
            });
            trunks.push(MeshInstance {
                pos: [t.x, sh, t.z],
                color: [0.66, 0.52, 0.34], // pale raw cut wood
                scale: [sr * 0.62, sh * 0.06, sr * 0.62],
                axis: UP,
            });
            continue;
        }
        // No special trunk here — the trunk is segment 0 of tree_branches
        // (the root limb) and is drawn by the loop below like any other
        // limb, so radius flows continuously from bole to tips and the
        // trunk→primary junction is a real fork, not two things overlapping.
        let element_r = h * sp.leaf_element_ratio;
        let cluster_r = h * sp.cluster_radius_ratio;
        // A weathered grey-brown for dead twigs, so a bare limb reads as
        // deadwood among the living branches.
        const DEAD_LIMB_COLOR: [f32; 3] = [0.34, 0.30, 0.25];
        // Pale raw wood at a break — a torn splinter where a limb snapped.
        const SPLINTER_COLOR: [f32; 3] = [0.72, 0.62, 0.45];
        // Dark mossy green that creeps up the shaded lower trunk.
        const MOSS_COLOR: [f32; 3] = [0.20, 0.42, 0.18];
        // A dark twiggy bird's nest wedged in a fork.
        const NEST_COLOR: [f32; 3] = [0.26, 0.19, 0.11];
        // Per-tree organic detail — rolled once so it's a trait of the
        // tree, not a per-limb speckle. Moss on the damp/shaded ones;
        // a nest only rarely; splinters only where deadwood already is.
        let mossy = leaf_hash01(seed, 0x0055_A100) < 0.35;
        let nesting = leaf_hash01(seed, 0x4E57_0000) < 0.09;
        let mut nest_placed = false;
        // Fruit: SOME trees of a fruiting species bear (a per-tree roll,
        // so a stand is a mix of laden and bare trees). Witch's snot on
        // the fungal species bears on nearly every tree; apples less so.
        let bears_fruit = sp.fruit_color.is_some()
            && leaf_hash01(seed, 0xF00D_0001) < if sp.fruit_on_dead_limbs { 0.95 } else { 0.6 };
        let mut leaf_i = 0u32;
        let mut tip_i = 0u32;
        for (i, seg) in tree_branches(seed, sp).into_iter().enumerate() {
            // Segment 0 is the ROOT (the trunk) — it starts at the ground,
            // so it is NOT seated into a parent (there isn't one).
            let is_root = i == 0;
            // The limb: the unit cone (base r=1, height 1 along +Y)
            // scaled to [radius, length, radius] and rotated +Y → axis.
            // A dead tip greys out — deadwood, not foliage. Girth scales
            // every limb uniformly (see above).
            let (bx, by, bz) = (t.x + seg.base[0] * h, seg.base[1] * h, t.z + seg.base[2] * h);
            let br = seg.base_radius * h * girth;
            // Seat the limb's open base INTO its parent: slide the base back
            // along -axis and lengthen to match, so the hollow (uncapped)
            // base ring is buried inside the parent's wood rather than
            // gaping at the junction. The tip is unchanged (base − a + (len
            // + a)·axis = base + len·axis), so foliage still sits at the tip.
            // The root has no parent, so it isn't seated.
            let seat = if is_root { 0.0 } else { br * 2.5 };
            // Wind sway weight: thinner limbs sway more. The root (radius =
            // trunk_radius) is 0 (rigid); a thin outer twig ~1 (flutters).
            // Leaves/fruit at a tip inherit it so they move with the twig.
            let sway = (1.0 - seg.base_radius / sp.trunk_radius).clamp(0.0, 1.0);
            let limb_axis = [seg.axis[0], seg.axis[1], seg.axis[2], sway];
            trunks.push(MeshInstance {
                pos: [bx - seg.axis[0] * seat, by - seg.axis[1] * seat, bz - seg.axis[2] * seat],
                color: if seg.is_dead { DEAD_LIMB_COLOR } else { sp.branch_color },
                scale: [br, seg.length * h + seat, br],
                axis: limb_axis,
            });
            // Moss creeps on the lower, shaded limbs of a mossy tree — a
            // few dark-green tufts clinging where a limb meets the bole.
            if mossy && seg.base[1] < 0.45 && leaf_hash01(seed, 0x0_5A00 ^ tip_i.wrapping_mul(2654435761)) < 0.35 {
                let mr = element_r * 1.6;
                let out = {
                    let l = (bx - t.x).hypot(bz - t.z).max(1.0);
                    [(bx - t.x) / l, 0.25, (bz - t.z) / l]
                };
                canopy_elements.push(MeshInstance {
                    pos: [bx, by, bz],
                    color: MOSS_COLOR,
                    scale: [mr, mr, mr],
                    axis: [out[0], out[1], out[2], 0.0], // moss clings low — rigid
                });
            }
            // A bird's nest wedges into one fork (an interior limb's tip)
            // in the upper-middle of a nesting tree — one per tree, rare.
            if nesting && !nest_placed && !seg.is_tip {
                let fork = seg.tip();
                if fork[1] > 0.4 && fork[1] < 0.85 {
                    let nr = element_r * 3.2;
                    canopy_elements.push(MeshInstance {
                        pos: [t.x + fork[0] * h, fork[1] * h, t.z + fork[2] * h],
                        color: NEST_COLOR,
                        scale: [nr, nr * 0.6, nr],
                        axis: UP,
                    });
                    nest_placed = true;
                }
            }
            if !seg.is_tip {
                continue;
            }
            let tip = seg.tip();
            let (wx, wy, wz) = (t.x + tip[0] * h, tip[1] * h, t.z + tip[2] * h);
            if seg.is_dead {
                // A dead twig grows no leaves. On the fungal species it's
                // where witch's snot clings — a fat round glob AT the tip
                // (not hanging), the sickly fruit-colour, on most dead tips
                // of a bearing tree. "Finding a mushroom", made visual.
                if let Some(fruit) = sp.fruit_color
                    && bears_fruit
                    && sp.fruit_on_dead_limbs
                    && leaf_hash01(seed, 0x5A0F_0000 ^ tip_i) < 0.8
                {
                    let gr = element_r * 3.0;
                    canopy_elements.push(MeshInstance {
                        pos: [wx, wy, wz],
                        color: fruit,
                        scale: [gr, gr, gr],
                        axis: [0.0, -1.0, 0.0, sway],
                    });
                } else if !sp.fruit_on_dead_limbs
                    && leaf_hash01(seed, 0x5311_0000 ^ tip_i) < 0.5
                {
                    // Not a fungal snot-bearer: some broken tips show pale
                    // raw wood — a splinter tear where the limb snapped, a
                    // small bright card capping the grey stub.
                    let sr = seg.base_radius * h * girth * 1.4;
                    canopy_elements.push(MeshInstance {
                        pos: [wx, wy, wz],
                        color: SPLINTER_COLOR,
                        scale: [sr, sr, sr],
                        axis: [seg.axis[0], seg.axis[1], seg.axis[2], sway],
                    });
                }
                tip_i += 1;
                continue;
            }
            // Live tip → a small Fibonacci-sphere ball of leaves so
            // foliage sits at the branch ends. Each leaf gets an autumn
            // age: mostly green (roll²), ceilinged by the species' autumn
            // — pine stays green, oak reddens, birch yellows.
            // Each cluster gets its OWN random spin, so no two tips are the
            // identical Fibonacci spray (without this, leaf k points the
            // same world direction in every cluster — a lattice of aligned
            // cards). Each leaf then gets a small directional jitter, so
            // even within a cluster the cards face — and roll — every which
            // way, like real foliage.
            let spin =
                leaf_hash01(seed, 0xC0FE_0000 ^ tip_i.wrapping_mul(2_654_435_761)) * std::f32::consts::TAU;
            for k in 0..sp.leaves_per_tip {
                let ky = 1.0 - 2.0 * (k as f32 + 0.5) / (sp.leaves_per_tip as f32);
                let kr = (1.0 - ky * ky).max(0.0).sqrt();
                let kt = (k as f32) * GOLDEN_ANGLE_RAD + spin;
                let roll = leaf_hash01(seed, leaf_i);
                let jx = leaf_hash01(seed, leaf_i.wrapping_mul(0x9E37_79B9)) - 0.5;
                let jy = leaf_hash01(seed, leaf_i.wrapping_mul(0x85EB_CA6B)) - 0.5;
                let jz = leaf_hash01(seed, leaf_i.wrapping_mul(0xC2B2_AE35)) - 0.5;
                leaf_i += 1;
                const JIT: f32 = 0.55; // directional jitter strength
                let mut dir = [
                    kr * kt.cos() + jx * JIT,
                    ky + jy * JIT,
                    kr * kt.sin() + jz * JIT,
                ];
                let dl = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt().max(1e-3);
                dir = [dir[0] / dl, dir[1] / dl, dir[2] / dl];
                let age = roll * roll * roll * sp.autumn;
                canopy_elements.push(MeshInstance {
                    pos: [
                        wx + cluster_r * dir[0],
                        wy + cluster_r * dir[1],
                        wz + cluster_r * dir[2],
                    ],
                    color: autumn_ramp(sp.leaf_green, age),
                    // Flat card: width (x) × length (z = width × aspect).
                    scale: [element_r, element_r, element_r * sp.leaf_aspect],
                    // Inherit the twig's sway so leaf and branch tip move
                    // together (same weight, same world point → lockstep).
                    axis: [dir[0], dir[1], dir[2], sway],
                });
            }
            // Hang a fruit at some LIVE tips of a bearing apple-type tree —
            // a small round card in the fruit colour, dropped just below
            // the tip and facing down so it reads as a hanging apple, not a
            // red leaf. (Dead-limb bearers grew their snot above.)
            if let Some(fruit) = sp.fruit_color
                && bears_fruit
                && !sp.fruit_on_dead_limbs
                && leaf_hash01(seed, 0x0FF0_0000 ^ tip_i) < 0.35
            {
                let fr = element_r * 2.2;
                canopy_elements.push(MeshInstance {
                    pos: [wx, wy - fr, wz],
                    color: fruit,
                    scale: [fr, fr, fr],
                    axis: [0.0, -1.0, 0.0, sway],
                });
            }
            tip_i += 1;
        }
    }
    MeshTreeInstances {
        trunks,
        canopy_elements,
        wood_verts: Vec::new(),
        wood_indices: Vec::new(),
    }
}

/// Same as `snapshot_to_mesh_instances`, plus the continuous woody
/// surface for every tree merged into one world-space vertex+index
/// buffer. Used by the native path (`render.rs`) — the browser path
/// still calls the trunks-only version until step 4.
///
/// Per tree: `tree_surface(seed, sp)` produces a unit-space mesh; we
/// scale by `height` (uniform, so normals are unchanged) and offset by
/// the tree's world position. Indices are rebased so every tree's
/// triangles reference into the merged vertex list.
pub fn snapshot_to_mesh_instances_with_wood(snap: &SceneSnapshot) -> MeshTreeInstances {
    let mut out = snapshot_to_mesh_instances(snap);
    let mut wood_verts: Vec<MeshVertex> = Vec::new();
    let mut wood_indices: Vec<u32> = Vec::new();
    for (t, h, sp, stump) in &snap.trees {
        if *stump {
            continue; // Stumps stay on the cone path — a stump ISN'T a full
            // tree skeleton, and `tree_surface` would still emit a bole.
        }
        let mesh = tree_surface_cached(tree_seed(t.x, t.z), sp);
        let (verts, indices) = (&mesh.0, &mesh.1);
        let base = wood_verts.len() as u32;
        for v in verts {
            wood_verts.push(MeshVertex {
                pos: [v.pos[0] * *h + t.x, v.pos[1] * *h + t.y, v.pos[2] * *h + t.z],
                normal: v.normal,
                uv: v.uv,
            });
        }
        for &i in indices {
            wood_indices.push(base + i);
        }
    }
    out.wood_verts = wood_verts;
    out.wood_indices = wood_indices;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_math::Vec3;

    fn tree_snapshot(pos: Vec3, sp: &'static crate::tree_mesh::TreeSpecies) -> SceneSnapshot {
        tree_snapshot_ex(pos, sp, false)
    }

    fn tree_snapshot_ex(
        pos: Vec3,
        sp: &'static crate::tree_mesh::TreeSpecies,
        stump: bool,
    ) -> SceneSnapshot {
        SceneSnapshot {
            trees: vec![(pos, 300.0, sp, stump)],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![],
            jukeboxes: vec![],
            player: Vec3::ZERO,
        }
    }

    #[test]
    fn wind_weight_is_zero_on_the_trunk_and_rises_on_thin_limbs() {
        use crate::tree_mesh::OAK;
        let m = snapshot_to_mesh_instances(&tree_snapshot(Vec3::new(500.0, 0.0, 500.0), &OAK));
        // The main bole is the first trunk instance — rigid (sway weight 0
        // in axis.w), so a breeze never bends the trunk.
        assert_eq!(m.trunks[0].axis[3], 0.0, "the trunk must not sway");
        // Thinner limbs carry real sway weight, and leaves inherit it so
        // foliage moves with its twig.
        assert!(m.trunks.iter().any(|i| i.axis[3] > 0.3), "thin limbs should sway");
        assert!(m.canopy_elements.iter().any(|e| e.axis[3] > 0.3), "leaves should sway");
    }

    #[test]
    fn some_apple_trees_bear_fruit_and_pines_never_do() {
        use crate::tree_mesh::{APPLE, PINE};
        let fruit = APPLE.fruit_color.expect("apple fruits");
        // Fruit are canopy elements pushed with the EXACT fruit colour;
        // leaves go through autumn_ramp and never land on it precisely.
        let fruit_count = |snap: &SceneSnapshot| {
            snapshot_to_mesh_instances(snap)
                .canopy_elements
                .iter()
                .filter(|e| e.color == fruit)
                .count()
        };
        let mut fruiting = 0;
        let n = 40;
        for k in 0..n {
            let pos = Vec3::new(k as f32 * 240.0, 0.0, -720.0);
            // No pine ever grows an apple.
            assert_eq!(fruit_count(&tree_snapshot(pos, &PINE)), 0);
            if fruit_count(&tree_snapshot(pos, &APPLE)) > 0 {
                fruiting += 1;
            }
        }
        // SOME apple trees bear, not all — an orchard is a mix.
        assert!(fruiting > 0, "no apple tree bore fruit");
        assert!(fruiting < n, "every apple tree bore fruit — expected a mix");
    }

    #[test]
    fn a_stump_is_a_cut_bole_of_its_species_with_no_crown() {
        use crate::tree_mesh::OAK;
        let pos = Vec3::new(500.0, 0.0, 500.0);
        let stump = snapshot_to_mesh_instances(&tree_snapshot_ex(pos, &OAK, true));
        let tree = snapshot_to_mesh_instances(&tree_snapshot(pos, &OAK));
        // A stump has NO crown — no leaf-card elements at all.
        assert!(stump.canopy_elements.is_empty(), "a stump has no foliage");
        // It keeps its species' bark (the bole's colour is OAK's trunk).
        assert_eq!(stump.trunks[0].color, OAK.trunk_color, "stump keeps species bark");
        // It's a short remainder, far shorter than the living tree's bole.
        assert!(
            stump.trunks[0].scale[1] < tree.trunks[0].scale[1] * 0.4,
            "a stump is a short remainder of the bole"
        );
        // And it shows a pale cut face on top.
        assert!(
            stump.trunks.iter().any(|i| i.color == [0.66, 0.52, 0.34]),
            "a stump shows a pale cut face"
        );
    }

    #[test]
    fn the_bole_reaches_every_primary_so_no_branch_floats() {
        // A primary that starts above where the trunk ends floats with a
        // see-through gap at the junction. The bole must rise to the
        // highest primary attachment for every species.
        use crate::tree_mesh::{tree_branches, BIRCH, APPLE, DEAD, FUNGAL, MAPLE, OAK, PINE, TreeSpecies, WILLOW};
        const SPECIES: [&TreeSpecies; 8] =
            [&PINE, &OAK, &BIRCH, &WILLOW, &APPLE, &MAPLE, &FUNGAL, &DEAD];
        let (px, pz, h) = (500.0_f32, 500.0_f32, 300.0_f32);
        for sp in SPECIES {
            let m = snapshot_to_mesh_instances(&tree_snapshot(Vec3::new(px, 0.0, pz), sp));
            let trunk_top = m.trunks[0].pos[1] + m.trunks[0].scale[1];
            let seed = tree_seed(px, pz);
            for seg in tree_branches(seed, sp) {
                // Primaries root on the trunk axis (x = z = 0 in unit space).
                let on_axis = seg.base[0].abs() < 1e-6 && seg.base[2].abs() < 1e-6;
                if on_axis {
                    assert!(
                        seg.base[1] * h <= trunk_top + 1.0,
                        "a primary at y={} floats above the bole top {trunk_top}",
                        seg.base[1] * h
                    );
                }
            }
        }
    }

    #[test]
    fn trees_wear_organic_detail_girth_moss_nests_splinters() {
        use crate::tree_mesh::OAK;
        const MOSS: [f32; 3] = [0.20, 0.42, 0.18];
        const NEST: [f32; 3] = [0.26, 0.19, 0.11];
        const SPLINTER: [f32; 3] = [0.72, 0.62, 0.45];
        let mut girths = Vec::new();
        let (mut mossy, mut nests, mut splinters) = (0, 0, 0);
        let n = 200;
        for k in 0..n {
            let pos = Vec3::new(k as f32 * 240.0, 0.0, 1500.0);
            let m = snapshot_to_mesh_instances(&tree_snapshot(pos, &OAK));
            // The main trunk is the first instance; its x-scale is girth.
            girths.push(m.trunks[0].scale[0]);
            let has = |c: [f32; 3]| m.canopy_elements.iter().any(|e| e.color == c);
            if has(MOSS) { mossy += 1; }
            if has(NEST) { nests += 1; }
            if has(SPLINTER) { splinters += 1; }
        }
        let (lo, hi) = girths.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        assert!(hi / lo > 1.8, "some trunks should be much fatter: {lo}..{hi}");
        assert!(mossy > 0 && mossy < n, "moss on SOME trees, not all: {mossy}/{n}");
        assert!(nests > 0 && nests < n / 3, "nests should be rare: {nests}/{n}");
        assert!(splinters > 0, "some broken tips should show a splinter: {splinters}");
    }

    #[test]
    fn fungal_grows_witches_snot_where_apples_and_pines_grow_none() {
        use crate::tree_mesh::{APPLE, FUNGAL, PINE};
        let snot = FUNGAL.fruit_color.expect("fungal bears witch's snot");
        // Snot is pushed with the EXACT fungal fruit colour; leaves (fungal
        // is evergreen-purple) and apple fruit (red) never land on it.
        let snot_count = |sp: &'static crate::tree_mesh::TreeSpecies| {
            let mut total = 0;
            for k in 0..60 {
                let pos = Vec3::new(k as f32 * 240.0, 0.0, 900.0);
                total += snapshot_to_mesh_instances(&tree_snapshot(pos, sp))
                    .canopy_elements
                    .iter()
                    .filter(|e| e.color == snot)
                    .count();
            }
            total
        };
        assert!(snot_count(&FUNGAL) > 0, "fungal trees should grow witch's snot");
        assert_eq!(snot_count(&APPLE), 0, "apple trees never grow snot");
        assert_eq!(snot_count(&PINE), 0, "pine trees never grow snot");
    }
}
