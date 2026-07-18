# Continuous wood

Trees are ONE surface — trunk, branches, roots — not a pile of instanced
cones. This is the arc: why, how, what's still undone.

## Why

The interior forks of the instanced-cone tree read as "cones sloppily
stacked" because they are: every limb is a separate primitive, so a fork
can't be continuous. A knuckle/callus to mask the seam was tried and
rejected — the seam is not to be masked; a fork must be **one continuous
organic surface**, and roots must **flare** from the base.

## How

Skin the woody skeleton as ONE **isosurface**:

- The wood's field = `smin` of round-cone SDFs, one per skeleton limb,
  plus five root cones flaring from `y = trunk_radius·1.5` down to
  `−trunk_radius·3.5` and outward.
- Polygonized by **marching tetrahedra**, not cubes: 6 tets per voxel,
  each with a handful of crossing cases — no 256-entry tables to
  mis-transcribe. Correct-by-inspection.
- Smooth-union blends every fork and the root flare organically, without
  stitching topology or callus.
- Leaves stay instanced cards — continuity doesn't matter for foliage.

`isosurface.rs` — primitives: `sd_capsule`, `sd_round_cone`, `smin`,
`Grid`, `marching_tetrahedra`, `emit_tri`, `process_cell`.
`tree_surface.rs` — `tree_surface(seed, sp) -> (Vec<MeshVertex>, Vec<u32>)`
composing them into one tree.

## Still undone

### Render integration

Draw the continuous wood mesh instead of the instanced cones.

- `MeshTreeInstances` carries `wood_verts: Vec<MeshVertex>` +
  `wood_indices: Vec<u32>` (WORLD space) alongside the leaf
  `canopy_elements: Vec<MeshInstance>` (unchanged).
- Draw the wood **once** with a single identity `MeshInstance`
  (`i_pos=0, i_scale=1, i_axis=[0,1,0,0]`) — the existing mesh pipeline
  + bark fragment work as-is (the wood carries UVs + normals). Touch
  `render.rs` (native), `render_web.rs` + `web/src/main.ts` (web): one
  extra indexed draw. Leaves keep instancing.

### Caching — MANDATORY before the browser sees it

Push auto-deploys game.sbvh.nl; ~0.3s/tree per frame would grind it.

- The unit-space surface is a pure fn of `(seed, species)` → cache it
  (`thread_local HashMap<(u32 seed, species_id), Rc<(verts,indices)>>`,
  size-capped or chunk-tied eviction so it doesn't leak as the player
  roams).
- Per frame: get-or-generate the local mesh, transform to world (×
  height + tree_pos; normals unchanged under uniform scale), append to
  the merged wood buffer. Only the cheap transform runs per frame; the
  isosurface runs once per unique tree.

### Known open problems

- **Wind on wood is lost** until a per-vertex sway weight (+ pivot) is
  baked into the wood vertex and the wood vs bends by it — the merged
  world mesh has no per-limb instance data. Leaves still sway. That
  same per-vertex channel is also what CURSOR interaction on thin
  branches needs.
- **Fine twigs** are floored to ~voxel radius (chunky). Trunk + major
  forks + roots are the payoff; twigs are leaf-covered.
- **First-appearance hitch**: even cached, a newly-visible tree costs
  ~0.3s to generate — may need async/background generation or lower res.

## Wanted, downstream

- **Cursor interaction** — leaves/thin branches react to the mouse
  moving past. Leaves: same shape as wind — push away from the cursor's
  world position, scaled by sway weight; the cursor rides the camera
  uniform like wind. Thin branches: easy if instanced, medium on the
  continuous wood (needs the per-vertex sway channel). BROWSER ONLY —
  seer can't verify it (no cursor).
- **Growth & decay as a life-stage axis**: `LifeStage` enum on
  `TreeTrunk` (Sapling·Mature·Snag·Stump·Fallen); retire the DEAD
  species → a `Snag` stage of any species (a dead oak keeps oak bark —
  same category-fix as stump); saplings (CDDA `t_tree_young` is
  dropped today); a decomposition scalar snag→punky→fallen log + root
  mound.
- **Interaction/simulation** (trees & fires are static render+collider):
  shake (drop fruit), chop, harvest ("find a mushroom" — snot is visual
  only), tree-falls-on-building, campfire burnout (`Campfire` has no
  fuel — burns forever).
- **Firelight** — no positional lights; the campfire lights nothing.
- **Audible wind** — via the existing `game_audio_play_samples`.
- **Species fidelity** — pinecones (pines bear nothing), pears render
  AS apples, lemons render as fruitless oaks.
- **Walls-on-mesh** (the branch's original north star) + a real sampled
  bark/leaf texture (bark is procedural).
