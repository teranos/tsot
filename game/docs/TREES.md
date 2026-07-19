# Trees

Trees are ONE surface — trunk, branches, roots — not a pile of instanced
cones. Fork continuity and root flare can't be masked, so the woody
skeleton is skinned as an isosurface: `smin` of round-cone SDFs
polygonized by marching **tetrahedra**. Leaves stay instanced cards.

One wood mesh is baked **per species, once**. Every oak instances the
same vertex+index buffer. Per-tree variation rides on `MeshInstance`.
Per-tree mesh generation was tried first and crashed the browser tab.

## Done

- Mesh pipeline (indexed instanced draws sharing one WGSL layout, macro
  pinned so `MESH_SHADER_WGSL` and `LEAF_SHADER_WGSL` can't drift apart).
- `INSTANCE_ATTRS` in `scene.rs` as the single source of truth for the
  52-byte `MeshInstance` layout, guarded by tests against `main.ts`.
- Continuous isosurface wood (`smin` + marching tetrahedra).
- One wood mesh per species, browser-shippable.
- Buttress roots flaring from the ankle.
- Pointy branch tips (`is_tip` cones taper to `rb = 0`).
- Continuous trunk taper — no beading at segment joints.
- Curved trunk per species (`trunk_curvature`) with a per-tree bend
  direction.
- Primary radius derived from local trunk radius at the attach point.
- Canopy positioned at the species skeleton's tips (canopy generation
  uses the same `SPECIES_SEED` the wood mesh is generated from).
- Leaves as a forward-facing tuft along the branch axis.
- Wind sway on wood and leaves, per-instance-weighted.
- Procedural bark shaded from a cylindrical UV. Leaf-card silhouette
  carved from the same UV so a rectangular quad renders as a leaf.
- Per-leaf autumn ramp (green → yellow → orange → red → brown, ceilinged
  by species `autumn` per-tree). Per-cluster phyllotactic spin so no
  two tufts are the same lattice.
- Per-tree girth, moss on lower shaded limbs, bird's nest wedged in a
  fork, splinters at broken tips, fruit at live tips, witch's snot at
  dead tips of the fungal species.
- Deadwood as a per-tree trait (some trees carry any at all, per
  species odds), re-rolled per limb for variety across a stand.
- Species differentiation for pine / oak / birch / willow / apple /
  maple / fungal / dead.
- CDDA authored-tree bridge (`t_tree_*` → `TreeKind` → `spawn_tree`).
  The apple orchard is the first authored stand.
- Stumps are a cut-state of any species (not their own species).

## Not done

- **Flat-cut crown top.** Trunk stops at `sp.base_y.1` with no tapered
  leader. Fix: emit a `is_tip=true` cone continuing past the top, or
  a real recursive leader child.
- **Every tree of a species leans the same way.** The species mesh
  bakes one bend direction. Fix: per-tree Y-rotation as an instance
  attribute, or N rotation-bucket meshes per species.
- **Fine twigs floored to voxel radius.** Chunky at the ends.
- **Trunk isn't depth-0 of the recursion.** The multi-segment trunk
  emit is an SDF-primitive artefact; semantically it's one limb.
  Unwind by keeping one `BranchSegment` and bending inside
  `collect_cones`.
- **`DEAD` is its own species.** Same category mistake as stump was;
  should be a `Snag` life-stage of any species (a dead oak keeps oak
  bark).
- **Life-stage axis** — `LifeStage` enum on `TreeTrunk`
  (Sapling · Mature · Snag · Stump · Fallen). Saplings (CDDA
  `t_tree_young` is dropped today). Decomposition scalar
  snag → punky → fallen log + root mound.
- **Per-tree tunable overrides** — right-click a tree to edit only
  that instance's shape params. Requires: tree ray-pick from pointer,
  per-instance param storage, per-tree mesh regeneration for edited
  trees (breaks the shared species mesh for those; the rest stay
  shared).
- **Cursor interaction** — leaves and thin twigs displace toward the
  pointer. Leaves are easy (same shape as wind, scaled by sway
  weight). Thin twigs need a per-vertex sway channel baked into
  `MeshVertex`.
- **Runtime branch-density knobs** — `branch_depth_add`,
  `branch_primaries_mult`. Same generation-invalidate path as the
  shipped wood-shape knobs.
- **Interaction/simulation** — shake (drop fruit), chop, harvest
  ("find a mushroom" — witch's snot is visual only today),
  tree-falls-on-building, campfire fuel + burnout (`Campfire` has no
  fuel and burns forever).
- **Firelight** — no positional lights; the campfire lights nothing.
- **Audible wind** — via the existing `game_audio_play_samples`.
- **Species fidelity** — pinecones (pines bear nothing), pears render
  AS apples, lemons render as fruitless oaks.
- **Walls-on-mesh** (the branch's original north star, see RENDER.md)
  + a real sampled bark/leaf texture (bark is procedural).

The knob list lives in `tune.rs::TuneParams`.
