# HANDOVER — `renderer/mesh-tree`

Branch tip (committed + pushed): **`275dc99`** (`game: isosurface core`).
Base: `master`.

This is the real handover — written assuming the session ends here.
There is **uncommitted, deliberately-not-committed work** (the
continuous-surface generator) that will be LOST when the session dies;
§3 specs it well enough to rebuild.

Verified: `cargo +nightly test --lib` → **115 pass** in the working
tree (fewer at the committed tip — see §3); native + wasm32 build. The
render is proven by seer's lavapipe frames at
`seer.sbvh.nl/perf/<sha>/frame-*.png`; the browser at game.sbvh.nl
(push auto-deploys). GPU output is only real once seer or the browser
shows it.

Code lives in: `scene.rs` (data model + cube/glass/ghost emit),
`shaders.rs` (WGSL), `tree_emit.rs` (instanced-cone tree emit),
`tree_mesh.rs` (skeleton + species), `isosurface.rs` (committed core),
and — uncommitted — `tree_surface.rs`.

---

## 1. What the branch contains (committed)

A full instanced-cone tree renderer, grown feature by feature:

- **Mesh substrate**: trees are real geometry (tapered trunk/branch
  cones + leaf cards) instanced through a mesh pipeline, not scaled
  cubes. Two draws sharing one WGSL layout (`mesh_layout_wgsl!`).
- **8 species** from one parametric `TreeSpecies` + a pure recursive
  `tree_branches` generator. Per-leaf autumn.
- **The trunk IS the root of the recursion** — segment 0 of
  `tree_branches`, radius flowing down by `radius_shrink`, so a branch
  can never be thicker than the trunk (proven by test). This retired the
  separate `trunk_r_ratio`/`primary_radius` that used to disagree. Same
  category-fix as stump/dead below.
- **Wind**: branches AND leaves sway. `MeshInstance.axis` is a `vec4` —
  `xyz` orientation, `w` per-instance sway weight (trunk 0/rigid, thin
  twig ~1); the mesh vs pivots each limb at its base (`× v.pos.y`),
  leaves inherit their twig's weight so foliage and branch move in
  lockstep. Rides the camera uniform — no new `env.*` crossing.
- **Weathering as data**: fruit (apples), dead limbs (`is_dead`, a
  per-tree trait via `dead_limb_odds`), witch's snot on fungal dead
  tips, bark (procedural, from the UV), per-tree girth, moss, bird's
  nests, splinters. **Stumps** = a cut-STATE of any species
  (`TreeTrunk.stump`), not a species — an oak stump keeps oak bark.
- **De-aligned foliage**: each leaf cluster gets a random spin + each
  leaf a jitter, so the canopy isn't a lattice of identical sprays.
- **CDDA authored-tree bridge**: `t_tree_*` → `cell_to_tree` →
  `TreeKind` → `Template.trees` → stamped → `species_for_kind` →
  `spawn_tree`. Payoffs: the apple orchard + every building's yard trees.
- **The instance-layout single source of truth** (`INSTANCE_ATTRS` in
  `scene.rs`): the 48→52-byte `MeshInstance` layout is pinned in one
  place; a test embeds `web/src/main.ts` and holds the hand-written JS
  offsets to it, so the one unguarded copy can't drift. No proto, no
  codegen.
- **`scene.rs` split** into `shaders.rs` + `tree_emit.rs` (was 1742
  lines).
- **Isosurface core** (tip `275dc99`): `isosurface.rs` — marching
  tetrahedra + `sd_capsule` + `smin` + gradient normals, with tests
  (sphere → closed shell; capsule bounded; two capsules fuse). This is
  the *committed* part of the continuous-surface work.

---

## 2. THE DECISION (why we're going to continuous wood)

The interior forks read as "cones sloppily stacked on top" because they
**are** stacked: every limb is a separate instanced cone, and separate
primitives share no surface, so a fork can't be continuous. A **knuckle
/ callus that masks the seam was tried and rejected** — the user does
not want the seam masked; a fork must be **one continuous organic
surface**, and roots must **flare** from the base (a big tree shows
structure where it meets the ground), which stuck-on cones also can't do.

**Decision: skin the woody skeleton as ONE continuous surface — an
isosurface.** Define the wood as a smooth-union of capsule/round-cone
distance fields along every limb (+ root cones flaring into the ground),
and polygonize the zero level set. Smooth-union (`smin`) makes every
fork and the root flare blend organically, automatically — no stitching
topology, no seam, no callus. Marching **tetrahedra** (not cubes) so
there are no 256-entry tables to mis-transcribe — it's verifiable.

This replaces the instanced-cone WOOD (trunk + branches + roots); LEAVES
stay instanced cards (continuity doesn't matter for foliage).

---

## 3. The uncommitted generator — WILL BE LOST — rebuild spec

Committed (`275dc99`): `isosurface.rs` core = `marching_tetrahedra`
(full grid), `sd_capsule`, `smin`, gradient normals, 3 tests.

**Uncommitted (dies with the session) — three files:**

### 3a. `isosurface.rs` additions
- **`sd_round_cone(p, a, b, ra, rb)`** — a tapered capsule (radius `ra`
  at `a`, `rb` at `b`), so a limb is tapering wood not a uniform dowel.
  Use Inigo Quilez's round-cone SDF (search "iq round cone sdf"): `ba =
  b−a; l2 = |ba|²; rr = ra−rb; a2 = l2−rr²; y = pa·ba; z = y−l2; x2 =
  |pa·l2 − ba·y|²; …` with the three-branch return (cap a, cap b, side).
- **Narrow-band `Grid`** (the perf fix — a uniform fine grid over a thin
  skeleton is ~64³ of mostly air and takes seconds):
  - `struct Grid { min:[f32;3], step:[f32;3], res:usize, cells:
    Option<&[[usize;3]]> }`.
  - `marching_tetrahedra(field, &Grid, vertex)` memoizes field per grid
    vertex in a `vec![f32::NAN; (res+1)³]` (NaN = not computed) and
    marches only `cells` when `Some`.
  - Extracted `process_cell` (the 6-tet 1/2/3-in triangulation) and
    `emit_tri` (winds the triangle so its face normal agrees with the
    field gradient → outward; per-vertex gradient normals → smooth).
  - `VertexFn = dyn Fn([f32;3],[f32;3]) -> MeshVertex` hook for UVs.

### 3b. `tree_surface.rs` — `tree_surface(seed, sp) -> (Vec<MeshVertex>, Vec<u32>)`
Pure, deterministic, unit tree space. Steps:
1. `segs = tree_branches(seed, sp)`.
2. **Roots**: 5 round-cones from `a=[0, trunk_radius·1.5, 0]` to
   `b=[cosθ·reach, −depth, sinθ·reach]`, `ra=trunk_radius·1.15`,
   `rb=trunk_radius·0.35`; `reach=trunk_radius·7`, `depth=trunk_radius·
   3.5`; `θ = seed_phase + i/5·TAU`. These smooth-union into the bole to
   flare the foot into buttress roots.
3. **Bounds**: every capsule endpoint ± radius, padded by
   `trunk_radius·2`, then cubed (uniform voxels).
4. **Resolution**: `res = clamp(ceil(span/(trunk_radius·0.9)), 20, 44)`;
   `voxel = span/res`; `rfloor = voxel·1.1` (floor every limb radius so
   fine twigs don't vanish between grid lines — they get chunky but stay
   continuous); `blend = voxel·1.2` (smin fillet radius).
5. **Field** = `smin` over all limb round-cones (`ra =
   base_radius.max(rfloor)`, `rb = (base_radius·radius_shrink)
   .max(rfloor)`) + the root cones, blend `blend`.
6. **Narrow band**: rasterize each capsule's AABB (expanded by
   `r+blend+voxel`) into grid cell indices → a `HashSet<[usize;3]>` of
   active cells.
7. **Bark UV** (cylindrical): `u = atan2(z,x)/TAU + 0.5`, `v = y·12`
   (the furrow pattern is periodic so the atan2 seam is invisible).
8. `marching_tetrahedra(field, &Grid{…, cells:Some(&cells)}, &vertex)`.

Cost: **~0.3s/tree release** (narrow-banded). Tests: bounded surface
with roots below y=0 + a trunk on the axis; determinism.

### 3c. `lib.rs`: `pub mod isosurface;` (committed) + `pub mod tree_surface;` (uncommitted).

---

## 4. The two UNDONE hard parts (the real remaining work)

**A. Render integration** — draw a continuous world-space wood mesh
instead of instanced cones.
- `MeshTreeInstances` carries `wood_verts: Vec<MeshVertex>` +
  `wood_indices: Vec<u32>` (WORLD space) alongside the leaf
  `canopy_elements: Vec<MeshInstance>` (unchanged).
- Draw the wood **once** with a single identity `MeshInstance`
  (`i_pos=0, i_scale=1, i_axis=[0,1,0,0]`) — the existing mesh pipeline +
  bark fragment work as-is (the wood carries UVs + normals). Touch
  `render.rs` (native), `render_web.rs` + `web/src/main.ts` (web): one
  extra indexed draw. Leaves keep instancing.

**B. Caching — MANDATORY before the browser sees it** (push
auto-deploys game.sbvh.nl; ~0.3s/tree per frame would grind it):
- The unit-space surface is a pure fn of `(seed, species)` → cache it
  (`thread_local HashMap<u32 seed, Rc<(verts,indices)>>`, size-capped or
  chunk-tied eviction so it doesn't leak as the player roams).
- Per frame: get-or-generate the local mesh, transform to world (`×
  height + tree_pos`; normals unchanged under uniform scale), append to
  the merged wood buffer. Only the cheap transform runs per frame; the
  isosurface runs once per unique tree.

**Known open problems with the continuous wood:**
- **Wind on wood is lost** until a per-vertex sway weight (+ pivot) is
  baked into the wood vertex and the wood vs bends by it — the merged
  world mesh has no per-limb instance data. Leaves still sway. That same
  per-vertex channel is also what CURSOR interaction on thin branches
  needs (§6).
- **Fine twigs** are floored to ~voxel radius (chunky). Trunk + major
  forks + roots are the payoff; twigs are leaf-covered.
- **First-appearance hitch**: even cached, a newly-visible tree costs
  ~0.3s to generate — may need async/background generation or lower res.

---

## 5. Seams / what's tested
- Instance layout: `INSTANCE_ATTRS` is the source of truth; three tests
  hold native + WGSL + JS to it (52 bytes, `i_axis` vec4 at 36).
- `mesh_layout_wgsl!` shared between MESH/LEAF; per-pipeline vertex
  stages; a test guards the shared layout.
- `TreeKind` framework-free in `crates/cdda`, tag pinned (Apple 0 …
  Stump 9).
- Isosurface: sphere→shell, capsule bounded, capsules fuse, round-cone
  tapers, tree skins with roots, determinism.
- 115 game lib tests (working tree) / cdda 31.

---

## 6. WANTED — not yet built
**Continuous wood** (§3–4) is the live front. Then:
- **Cursor interaction** — leaves/thin branches react to the mouse
  moving past. Leaves: EASY (same shape as wind — push away from the
  cursor's world position, scaled by sway weight; the cursor rides the
  camera uniform like wind, no new crossing; JS unprojects mousemove to
  the ground plane). Thin branches: easy if instanced, MEDIUM on the
  continuous wood (needs the per-vertex sway channel from §4). BROWSER
  ONLY — seer can't verify it (no cursor); the one feature the frame
  can't prove.
- **Growth & decay as a life-stage axis**: `LifeStage` enum on
  `TreeTrunk` (Sapling·Mature·Snag·Stump·Fallen); **retire the `DEAD`
  species → a `Snag` stage of any species** (a dead oak keeps oak bark —
  same category-fix as stump); **saplings** (CDDA `t_tree_young` is
  dropped today); a **decomposition scalar** snag→punky→fallen log +
  root mound.
- **Interaction/simulation** (trees & fires are static render+collider):
  shake (drop fruit), chop, harvest ("find a mushroom" — snot is visual
  only), tree-falls-on-building, campfire burnout (`Campfire` has no
  fuel — burns forever).
- **Firelight** — no positional lights; the campfire lights nothing.
- **Audible wind** — via the existing `game_audio_play_samples` (no new
  crossing).
- **Species fidelity** — pinecones (pines bear nothing), pears render
  AS apples, lemons render as fruitless oaks.
- **Walls-on-mesh** (the branch's original north star) + a real sampled
  bark/leaf texture (bark is procedural).

---

## 7. Housekeeping
- **The `tree_surface` generator is NOT committed by decision** — it
  dies with this session; rebuild from §3.
- A **stale background seer-poll** was left running (from the reverted
  knuckle commit) — moot, dies with the session.
- **Open the PR** when ready — squash intent-sized; scrub any CC-BY-SA
  CDDA blobs. No PR exists yet.
