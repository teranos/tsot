# HANDOVER — `renderer/mesh-tree`

Branch tip: `630a4b7` (pushed). Base: `master`. 34 commits.

Trees left the cube path and became real geometry: tapered-cone limbs
and flat leaf cards, instanced through a mesh pipeline, speciated into
eight forms, wind-swayed (branches *and* leaves), bark-textured,
weathered with deadwood / moss / nests / splinters / stumps — and, the
payoff, placed from CDDA maps as the species the map names, through the
same stamp pipeline that places buildings.

Verified two ways: `cargo +nightly test` (**game 108** lib tests, **cdda
31**) and native **and** wasm32 libs build clean with `-D warnings`; the
*render* is verified by seer's lavapipe frames at
`seer.sbvh.nl/perf/<sha>/frame-*.png`. GPU output is only real once seer
(or the browser) shows it — no `cargo test` proves a pixel.

The code lives in three files now (was one 1742-line `scene.rs`):
`scene.rs` (data model + cube/glass/ghost emit), `shaders.rs` (all
WGSL), `tree_emit.rs` (the whole tree emit — the churn source).

---

## What the branch contains

**1. Mesh render substrate.** Baked once: a unit trunk cone
(`trunk_mesh`) + a double-sided leaf quad (`leaf_quad_mesh`). Two draws,
two pipelines sharing one WGSL **layout** (`mesh_layout_wgsl!`):
- **MESH** — trunk + branch cones; vertex stage sways each limb by its
  weight; fragment paints **procedural bark** from the UV.
- **LEAF** — leaf cards; vertex stage sways by the full weight; fragment
  carves a procedural almond silhouette (`discard`) + two-sided light.
One packed instance buffer per draw; `first_instance` slices trunks
from canopy elements.

**2. Wind — branches AND leaves (`630a4b7`).** `MeshInstance.axis` is a
`vec4`: `xyz` orientation, **`w` = per-instance sway weight**. Both
vertex stages sway via one shared `wind_offset`; the mesh stage
multiplies by `v.pos.y` so each limb pivots at its base (thin outer
twigs flutter most). Weight `= 1 − base_radius/primary_radius`: trunk 0
(rigid), thin twig ~1. Leaves/fruit inherit their twig's weight, so
foliage and branch move in lockstep. Time = `camera.wind.x` (elapsed
seconds, synthetic ticks — no `bevy_time`). Rides the existing camera
uniform + instance buffer — **zero new `env.*` crossings**.

**3. Eight species from one generator.** `TreeSpecies` (~27 fields);
`tree_branches(seed, &TreeSpecies)` is a pure recursive generator.
PINE · OAK · BIRCH · WILLOW · APPLE · MAPLE · FUNGAL · DEAD. Per-leaf
autumn (`autumn_ramp`) keeps most green, a few turning. Species is DATA
on the tree (`TreeTrunk { height, species, stump }`), never re-guessed.

**4. Weathering & detail — all as per-tree data.**
- **Fruit**: apples hang below LIVE tips, ~⅓ of tips, ~60% of trees.
- **Bigger apples**: `authored_scale = 1.3` (authored-height aware).
- **Dead limbs**: `is_dead` on a segment; a per-TREE trait
  (`dead_limb_odds`), not a uniform speckle; dead tips grey + bare.
- **Witch's snot**: FUNGAL grows sickly-green globs at its dead tips
  (`fruit_on_dead_limbs`) — the visual half of "find a mushroom".
- **Trunk girth**: per-tree factor, a few trees notably fat.
- **Moss / bird's nests / splinters**: per-tree rolls — moss on lower
  limbs, a rare nest in one fork, pale torn wood at broken tips.
- **Stumps**: `TreeTrunk.stump` — the short remainder of a felled tree
  of ITS species (an oak stump keeps oak bark) + a pale cut face, no
  crown. ~6% of forest trees; CDDA `t_stump` → cut oak. A cut-STATE, not
  a species (the right shape — see *Wanted* for extending it).
- **Junctions sealed**: the bole rises to the highest primary and each
  limb is seated into its parent, so you can't see through a joint.

**5. The CDDA authored-tree bridge — the arc's north star.**
`t_tree_apple` → `cell_to_tree` → `TreeKind::Apple` → `Template.trees`
→ stamped by `stream_chunks` → `species_for_kind` → `spawn_tree`.
Payoffs: the **apple orchard** (`assets/buildings/orchard.json`, 5×5,
wide clearing so rows read) and **every building's yard trees** for
free. Seer's tour finds the orchard and captures it at rest.

---

## Seams (where to be careful)

### The instance layout — now GUARDED by one source of truth
`MeshInstance` (`#[repr(C)]`, **52 bytes**, fields 0/12/24/36, `axis` a
vec4 at 36) is described in four places. `scene.rs::INSTANCE_ATTRS` is
the one source the other three answer to:
- **native** (`render.rs`) — `vertex_attr_array!`, cross-checked by
  `native_mesh_instance_attrs_derive_from_the_source`.
- **WGSL** (`shaders.rs`) — `@location` 3/4/5/6.
- **JS** (`web/src/main.ts`) — hand-written offsets, held to the const
  by `web_shim_mesh_instance_layout_matches_this_const` (embeds main.ts,
  parses the mesh instance block, asserts every offset/format). This was
  the ONE unguarded copy (seer only renders native); now JS drift is a
  red game-tests gate, not a browser surprise. No proto, no codegen.
- `instance_attrs_match_the_repr_c_struct` ties the const to the struct.

### One WGSL layout, per-pipeline vertex stages
`mesh_layout_wgsl!` holds the shared ABI (Camera incl. `wind`,
VIn/IIn/VOut, `basis_from_axis`, `wind_offset`, consts). MESH and LEAF
each `concat!` it with their own vertex + fragment. Rigidity is
per-instance (`axis.w`), NOT a shader that lacks wind.
`mesh_and_leaf_shaders_share_one_layout` guards against drift.

### `TreeKind` is framework-free
In `crates/cdda`, a thin tag — no bevy. Game owns `TreeKind →
&'static TreeSpecies`. `tree_kind_tag` **pinned** (Apple 0 … Stump 9);
`stable_digest`/`rotate_template` mix trees.

### The camera uniform carries `wind`
`Camera { view_proj, wind: vec4 }`, `GpuCamera` 80 bytes both backends;
non-mesh shaders read only `view_proj`. Web sources seconds from
`FrameCount`, native passes a per-tour-stop phase.

---

## What's tested

- **cdda (31):** `every_shipped_building_resolves_deterministically`
  (resolve twice, require equal), `orchard_resolves_to_a_grid_of_apple_
  trees`, `cell_to_tree`, digest/rotation over trees.
- **game (108):** the seam trio (struct-match / JS-contract / native
  cross-check); `wind_is_a_shared_offset_weighted_per_instance` +
  `wind_weight_is_zero_on_the_trunk_and_rises_on_thin_limbs`;
  `deadwood_is_a_per_tree_trait_bounded_by_species_odds`;
  `fungal_grows_witches_snot_...`; `a_stump_is_a_cut_bole_of_its_species
  _with_no_crown`; `the_bole_reaches_every_primary_so_no_branch_floats`;
  `trees_wear_organic_detail_...`; `mesh_and_leaf_shaders_share_one_
  layout`; species/branch/tree_at_cell contracts.

Not coverable by `cargo test`: that the GPU draws it. seer's frame is
the proof.

---

## WANTED — not yet built (things asked for, still missing)

Grouped by theme; each is a real gap, honestly marked. The single
biggest one is first.

### A. Growth & decay as a life-stage axis (the next big refactor)
The insight from stumps generalizes: **a tree is `species` × `stage`**.
Species = identity (bark, leaf, form); stage = where it is in life/death.
- **`LifeStage` enum** — Sapling · Mature · Snag · Stump · Fallen — on
  `TreeTrunk`, replacing the scattered `stump: bool`. NOT built.
- **Deprecate the `DEAD` *species*.** DEAD-as-a-species is the same
  category mistake stump was: a dead tree is a dead *oak* / dead *pine*
  that keeps its bark. Wanted as `stage = Snag` on any species. NOT
  built (DEAD is still a fake species in the table).
- **Saplings / young.** CDDA `t_tree_young` currently maps to `None`
  (dropped — no geometry). Wanted as a Sapling stage (small
  `authored_scale`, few branches, `dead_limb_odds = 0`). NOT built.
- **Decomposition as a scalar** `[0,1]`: snag → bark sloughing (patchy
  two-tone) → punky (heavy moss + fungal) → fallen log + root mound.
  The visuals already exist as points on this curve (deadwood, moss,
  snot, cut face); a decay axis would sequence them so a forest shows
  living trees, snags, a rotting log — a succession, not a scatter. NOT
  built. (The user called this the interesting part.)

### B. Interaction / simulation — trees & fires as STATEFUL, not props
Today a tree/fire is render + AABB collider only. No mutable state, no
action system. This blocks everything below:
- **Shake a tree** → drop its fruit. (Smallest.) NOT built.
- **Chop a tree** → health + a log drop. NOT built.
- **Harvest** — "find a mushroom": the witch's snot and apples are
  VISUAL only; you cannot pick them. NOT built.
- **Tree falls on a building** — needs physics + damaging the stamped
  building props (static instances today). (Largest.) NOT built.
- **Campfire burnout** — `Campfire { intensity }` has no fuel/lifetime,
  so it burns forever (the user asked why). Wanted: fuel decremented per
  tick, feeding it as an interaction. NOT built.

### C. Lighting & atmosphere
- **Firelight** — a campfire sheds NO light on nearby trees (asked why).
  There are no positional lights; every fragment uses one fixed
  directional light, and the fire is a self-orange cube that lights
  nothing. Wanted: a point-light term fed the fire's pos/intensity + a
  darker/night ambient so the warm pool means something. NOT built.
  (Biggest single payoff for "the world feels alive".)

### D. Sound
- **Audible wind** — the leaves/branches rustling in the wind. NOT
  built. Feasible via the EXISTING `game_audio_play_samples` (the
  campfire crackle uses it) — a volume-modulated rustle, no new
  `env.*` crossing.

### E. Species / botany fidelity
- **Pinecones** — a pine's "fruit" is a cone (gymnosperm, not a fruit);
  pines currently bear nothing. NOT built.
- **Pears distinct from apples** — `t_tree_pear` renders AS an apple
  today (folded onto `TreeKind::Apple`). Wanted its own form (yellow-
  green). NOT built.
- **Lemons / citrus** — unmapped, so a lemon renders as a fruitless oak
  (`Generic → OAK`). NOT built.
- **Fungal in the wild** — fungal trees never spawn procedurally (only
  CDDA-authored `t_tree_fungal`); `species_for` has no FUNGAL bucket. If
  rare-but-present in wild woods is wanted, that's one line. NOT built.
- (Botany note: walnut/pecan/hazelnut/chestnut are all botanically
  fruits, so the coarse "fruit/nut" bucket is fine as a *category* — it's
  appearance that should vary. CDDA has no taxonomy: flat `t_tree_<name>`
  ids, which is why `cell_to_tree` pattern-matches substrings.)

### F. Materials / the ORIGINAL north star
- **Real sampled bark/leaf texture** — bark is procedural (in-shader)
  today. A sampled image is the next step up in readability and is the
  first real new `env.*` crossing (a texture + sampler binding). NOT
  built.
- **Walls-on-mesh** — the branch's *original* north star
  (`game/docs/RENDER.md`). Trees proved the mesh substrate + the UV
  slot; walls are why UVs were laid down day-one. The doubled-edge
  wall-junction limitation is a mesh problem. NOT built.

---

## Done this session that was on earlier want-lists
Branch wind · stumps · moss · bird's nests · broken-branch splinters ·
bigger/varied trunks · procedural bark · dead limbs · witch's snot ·
bigger apples + fruit · the see-through junction fix · species-aware
authored height · the instance-layout single-source-of-truth (JS now
guarded) · the `scene.rs` split.

---

## Housekeeping
**Open the PR** — squash intent-sized; scrub any CC-BY-SA CDDA blobs
before it's public. No PR exists yet (none requested).
