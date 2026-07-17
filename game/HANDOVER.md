# HANDOVER — `renderer/mesh-tree`

Branch tip: `170492e` (pushed). Base: `master`. 21 commits.

Trees left the cube path and became real geometry: tapered-cone limbs
and flat leaf cards, instanced through a new mesh pipeline, speciated
into eight distinct forms, and — the payoff — placed from CDDA maps as
the species the map names, through the same stamp pipeline that places
buildings.

Everything here is verified two ways: `cargo +nightly test` (game 94
lib tests, cdda 32) and native builds clean; the *render* is verified
by seer's lavapipe frames at `seer.sbvh.nl/perf/<sha>/frame-*.png`, not
by intuition. GPU output is only real once seer (or the browser) shows
it.

---

## What the branch contains

**1. A mesh render substrate for trees.** Trees no longer emit two
scaled cubes. They emit real geometry drawn indexed-and-instanced:

- Baked once per render: a unit trunk cone (`trunk_mesh`) and a
  double-sided leaf quad (`leaf_quad_mesh`), both in `tree_mesh.rs`.
- Two draws per frame, two pipelines sharing the WGSL vertex stage:
  - **MESH pipeline** — trunk + every branch limb, as tapered cones.
  - **LEAF pipeline** — leaf cards, with a procedural almond
    silhouette (`discard`) and two-sided lighting.
- One packed instance buffer per draw; `first_instance` slices trunks
  from canopy elements so a single dispatch pulls its own range.

**2. Per-instance orientation.** A limb points where it grew; a leaf
faces its scatter direction. That needed orientation to cross three
boundaries byte-for-byte identically (see *Seams*).

**3. Eight tree species from one generator.** `TreeSpecies` is a ~20
field parameter struct; `tree_branches(seed, &TreeSpecies)` is a pure
recursive branch generator. The same code makes:

  PINE · OAK · BIRCH · WILLOW · APPLE · MAPLE · FUNGAL · DEAD

DEAD is a bare skeleton (`leaves_per_tip = 0`). FUNGAL is purple with
no autumn. MAPLE runs full autumn. APPLE is short and round. Per-leaf
autumn (`autumn_ramp` + `leaf_hash01`, `age = roll³ × species.autumn`)
puts most leaves green, a few going yellow→orange→red→brown, the way
the reference photo reads.

**4. Species is DATA on the tree, not a render-time guess.**
`TreeTrunk { height, species: &'static TreeSpecies }`. Procedural
forest trees fill `species` from `species_for_pos`. Authored CDDA
trees fill it from the species their map names. The renderer never
re-guesses.

**5. The CDDA authored-tree bridge — the north star of this arc.**
CDDA authors trees as `t_tree_*` terrain. That terrain now flows all
the way to a rendered, speciated tree:

  `t_tree_apple` (JSON)
    → `cell_to_tree` → `TreeKind::Apple`            (crates/cdda)
    → `TreePlacement { offset, kind }` on `Template.trees`
    → rotated + stamped by `stream_chunks` like any prop
    → `species_for_kind(kind)` → `&APPLE`            (game)
    → `spawn_tree` → mesh render

Two payoffs fell out of this:
  - **An apple orchard** — `crates/cdda/assets/buildings/orchard.json`,
    a 13×13 mapgen (5×5 = 25 `t_tree_apple`, dirt aisles), enrolled as
    a "building" with empty props. It gets a wide clearing
    (`ORCHARD_YARD_MARGIN = 380`) so the rows read as open rows, and
    short near-uniform `authored_height` (260–300) so crowns line up.
  - **Every building's yard trees** — CDDA house/garage mapgens carry
    their own `t_tree_*`; those now grow too, for free.

**6. Seer captures the orchard.** `seer_tour_from` finds an orchard
stop (identified as `props.is_empty() && !trees.is_empty()`); the tour
takes one frame per stop *at rest* (end-of-window), and seer.yml
uploads all `frame-*.png`.

Perf held across the whole arc: seer `[perf]` stayed in the
~40–54 µs band; orchard frame ~28–30 µs.

---

## Seams (where to be careful)

### The 48-byte oriented instance — one layout, three languages
`MeshInstance { pos:[f32;3], color:[f32;3], scale:[f32;3], axis:[f32;3] }`,
`#[repr(C)]`, 48 bytes. `axis` rotates the baked geometry's local +Y
onto the instance direction via `basis_from_axis` in WGSL. The layout
is declared in three places and MUST agree exactly:

| where | how |
|---|---|
| WGSL (`scene.rs`) | instance attrs at `@location` 3/4/5/6; axis at **6** |
| native (`render.rs`) | `vertex_attr_array![3=>Float32x3,…,6=>Float32x3]`, stride `size_of::<MeshInstance>()` |
| JS (`web/src/main.ts`) | `{ shaderLocation:6, offset:36, format:'float32x3' }`, `instanceStride:48` |

Change one field and all three move together, or trunks/leaves point
the wrong way in exactly one of {native, browser} — and only seer or a
real browser will show which.

### MESH vs LEAF pipeline
Same vertex stage, different fragment. LEAF adds the silhouette
`discard` (`half_w = 0.5*sin(π·uv.y)`) and two-sided lighting
(`abs(dot(n,l))`). Native builds both from one descriptor closure
(`make_mesh_pipeline`); web mirrors it (`RenderWebState.leaf_pipeline`).
The canopy draw sets the leaf pipeline before dispatching.

### `TreeKind` is framework-free
`TreeKind` lives in `crates/cdda` and is a thin tag — no bevy, no
`&'static`. The game owns the `TreeKind → &'static TreeSpecies` map
(`species_for_kind`). `tree_kind_tag` is **pinned** (Apple 0 … Dead 8);
`stable_digest` and `rotate_template` both mix trees, so a map's tree
placement is stable under rotation and across peers.

### The stamp path already handled props; trees rode in alongside
`Template` grew a `trees: Vec<TreePlacement>` field. That touched every
`Template { props }` literal (template/placement/campfire/campsite/
chunk). `footprint_half` and yard margins fold over `trees` too.

---

## What's tested

- **cdda (32):** `every_shipped_building_resolves_deterministically`
  (resolve twice, require equal — this replaced the old golden-master
  hash-snapshot; assert the invariant, don't pin the derived value),
  `orchard_resolves_to_a_grid_of_apple_trees` (25 trees, all Apple, no
  props), `cell_to_tree` mapping, digest/rotation over trees.
- **game (94):** `every_species_stays_in_unit_space_with_tips_in_the_crown`
  (all 8 species), `branch_recursion_terminates_for_every_species`,
  `species_pick_is_deterministic_and_varied`,
  `species_shape_the_tree_differently`,
  `leaf_quad_is_a_flat_double_sided_card`, plus `tree_at_cell`
  determinism/clearing/height tests.

Not coverable by `cargo test`: that the GPU actually draws it. That's
seer's job — the frame PNG is the proof.

---

## Open threads (next session)

1. **Walls-on-mesh** — the branch's *original* north star (see
   `game/docs/RENDER.md`). Trees proved the mesh substrate + the UV
   slot (dark code today, used only for the leaf silhouette). Walls are
   why UVs were laid down day-one: brick/damage/poster textures need a
   real sampled texture, and the doubled-edge wall-junction limitation
   is a mesh problem, not a placement one.
2. **Species-aware authored height.** `authored_height` is uniform for
   *all* authored trees — right for an apple orchard, wrong for a house
   with a tall oak in the yard. Height should key off species (short
   apple vs tall oak).
3. **A real leaf texture.** The UV is currently spent only on the
   procedural almond silhouette. A sampled leaf image is the next step
   up in readability (and exercises the same UV path walls will use).
4. **More `t_tree_*` mappings.** `cell_to_tree` folds many CDDA species
   onto eight; widen as new geometry lands (currently non-tree / stump
   / young → None).
5. **Open the PR.** Squash intent-sized; scrub any CC-BY-SA CDDA blobs
   before it's public. No PR exists yet (none requested).
