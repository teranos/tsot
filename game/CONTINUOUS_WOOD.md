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
composes them into one tree.

## Canonical mesh per species

The shippable shape: **one wood mesh per species, generated on first
sight, instanced per tree.** Every oak in the world uses the same
underlying vertex+index buffer; per-tree variation (position, height,
species tint) rides on the `MeshInstance`.

- `tree_surface::species_wood_mesh(sp)` — thread-local cache, at most
  one entry per species (~8), no eviction. Called at most 8 times over
  the process lifetime.
- `MeshTreeInstances.wood_by_species: Vec<(&'static TreeSpecies, Vec<MeshInstance>)>`.
- Renderer draws one indexed instanced call per species. `MeshInstance`
  carries `pos` (tree world position), `scale` (uniform height), `color`
  (species trunk_color), `axis = [0,1,0,0]` (identity rot, no wind).

The prior designs that DID NOT work:
- Per-tree mesh cache (unbounded generation as player roams).
- Byte-capped FIFO cache (thrashed; wasmtime hit 15/25-min timeout).
- Per-frame merge memoizer (retained the last merged buffer, ~350 MB
  Δheap on seer).

All of them scaled with unique-trees-visited. Canonical scales with
species count — a fixed small constant. Wasm heap flat at 1.55 MB
steady, generation cost paid once at first sight.

Sacrifice: every oak has the same trunk-and-branch silhouette. Girth,
moss, deadwood, autumn tint still per-instance on the cone/canopy path.

## Still undone

- **Wind on wood** — the canonical mesh instance uses `axis.w = 0` so
  the wood doesn't sway. Adding sway needs a per-vertex sway weight
  baked into `MeshVertex` (thicker limbs → lower weight, thin twigs →
  high weight) and a shader that pivots each limb at its base. The
  same per-vertex channel is what cursor interaction on thin branches
  will need.
- **Fine twigs** are floored to ~voxel radius (chunky). Trunk + major
  forks + roots are the payoff; twigs are leaf-covered.

## Wanted, downstream

- **Cursor interaction** — leaves/thin branches react to the mouse
  moving past. Needs the per-vertex sway channel (see above).
- **Growth & decay as a life-stage axis**: `LifeStage` enum on
  `TreeTrunk` (Sapling·Mature·Snag·Stump·Fallen); retire the DEAD
  species → a `Snag` stage of any species (a dead oak keeps oak bark
  — same category-fix as stump); saplings (CDDA `t_tree_young` is
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
