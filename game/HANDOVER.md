# HANDOVER — `renderer/mesh-tree`

Branch tip: `7531605` (pushed). Base: `master`. 26 commits.

Trees left the cube path and became real geometry: tapered-cone limbs
and flat leaf cards, instanced through a new mesh pipeline, speciated
into eight distinct forms, wind-swayed, materialed with procedural
bark — and, the payoff, placed from CDDA maps as the species the map
names, through the same stamp pipeline that places buildings.

Everything here is verified two ways: `cargo +nightly test` (game 101
lib tests, cdda 32) and native **and** wasm32 libs build clean; the
*render* is verified by seer's lavapipe frames at
`seer.sbvh.nl/perf/<sha>/frame-*.png`, not by intuition. GPU output is
only real once seer (or the browser) shows it — no `cargo test` can
prove a pixel.

---

## What the branch contains

**1. A mesh render substrate for trees.** Trees emit real geometry
drawn indexed-and-instanced:

- Baked once per render: a unit trunk cone (`trunk_mesh`) and a
  double-sided leaf quad (`leaf_quad_mesh`), both in `tree_mesh.rs`.
- Two draws, two pipelines sharing one WGSL **layout**:
  - **MESH pipeline** — trunk + every branch limb, tapered cones, a
    rigid vertex stage, a fragment that paints **procedural bark** from
    the UV (vertical furrows + lengthwise grain).
  - **LEAF pipeline** — leaf cards, a vertex stage that adds **wind
    sway**, a fragment with a procedural almond silhouette (`discard`)
    and two-sided lighting.
- One packed instance buffer per draw; `first_instance` slices trunks
  from canopy elements so one dispatch pulls its own range.

**2. Per-instance orientation.** A limb points where it grew; a leaf
faces its scatter direction. `axis` rotates the baked geometry's local
+Y via `basis_from_axis`. Crossing three languages byte-identically —
see *Seams*.

**3. Eight species from one generator.** `TreeSpecies` is a ~25-field
parameter struct; `tree_branches(seed, &TreeSpecies)` is a pure
recursive branch generator. Same code →
PINE · OAK · BIRCH · WILLOW · APPLE · MAPLE · FUNGAL · DEAD.
DEAD is a bare skeleton. FUNGAL is purple, evergreen. MAPLE runs full
autumn. APPLE is short, round, a touch bigger (`authored_scale`).
Per-leaf autumn (`autumn_ramp`, `age = roll³ × species.autumn`) keeps
most leaves green, a few turning.

**4. Species is DATA on the tree, not a render-time guess.**
`TreeTrunk { height, species: &'static TreeSpecies }`. Procedural trees
fill it from `species_for_pos`; authored CDDA trees from the species
their map names. The renderer never re-guesses.

**5. Fruit, deadwood, and snot — appearance as species data.**
- **Apples**: `fruit_color = Some(red)`, hung below LIVE tips on ~a
  third of tips, on ~60% of apple trees (a per-tree roll — an orchard
  is a mix of laden and bare trees). APPLE is slightly bigger via
  `authored_scale = 1.3`.
- **Dead limbs**: `BranchSegment.is_dead`. A per-TREE trait
  (`dead_limb_odds` = per-species probability a given tree bears any
  deadwood, rolled once off the seed), not a uniform speckle; within a
  bearing tree each tip dies with `DEAD_TIP_CHANCE`. Dead tips grey out
  and grow no leaves. A sapling species sets `dead_limb_odds = 0.0` so
  young trees are never gnarled.
- **Witch's snot**: FUNGAL has `fruit_color = Some(sickly-green)` +
  `fruit_on_dead_limbs = true`, so it clings as fat globs AT the dead
  tips of nearly every fungal tree — the visual half of "find a
  mushroom" (the harvest half is the interaction gap, below).

**6. Wind.** Leaf cards sway on a time-driven horizontal offset,
phased by world position so the canopy ripples rather than sliding as a
slab. Time is `camera.wind.x` = elapsed seconds (synthetic ticks —
`FrameCount × TICK_SECONDS`, no `bevy_time`, same model as the campfire
flicker). It rides the EXISTING camera uniform (Camera grew a
`wind: vec4`, GpuCamera 64→80 bytes) — **zero new `env.*` crossings**.
Trunks/branches are still rigid (branch wind is the next step — it
needs a per-instance sway field; see *Open threads*).

**7. The CDDA authored-tree bridge — the north star of this arc.**

  `t_tree_apple` (JSON)
    → `cell_to_tree` → `TreeKind::Apple`            (crates/cdda)
    → `TreePlacement { offset, kind }` on `Template.trees`
    → rotated + stamped by `stream_chunks` like any prop
    → `species_for_kind(kind)` → `&APPLE`            (game)
    → `spawn_tree` → mesh render

Two payoffs: the **apple orchard**
(`crates/cdda/assets/buildings/orchard.json`, 5×5 `t_tree_apple`, dirt
aisles, `ORCHARD_YARD_MARGIN = 380` clearing so rows read as open
rows), and **every building's yard trees** (CDDA mapgens carry their
own `t_tree_*`) — for free.

**8. Seer captures the orchard.** `seer_tour_from` finds an orchard
stop (`props.is_empty() && !trees.is_empty()`); one frame per stop at
rest; seer.yml uploads all `frame-*.png`. Perf held ~40–54 µs across
the arc; orchard frame ~28–30 µs.

---

## Seams (where to be careful)

### The oriented instance — one layout, three languages (THE hazard)
`MeshInstance { pos, color, scale, axis }`, `#[repr(C)]`, 48 bytes,
fields at 0/12/24/36. The layout is described four times; two are
coupled, two are hand-copied:

| where | how | coupled? |
|---|---|---|
| Rust struct (`scene.rs`) | `#[repr(C)]`, the PRODUCER of the bytes | source |
| native (`render.rs`) | `vertex_attr_array![3..6 => Float32x3]`, stride `size_of` | derived from field order |
| WGSL (`scene.rs`) | `@location` 3/4/5/6 | **hand-copied** |
| JS (`web/src/main.ts`) | `{shaderLocation:6, offset:36, format:'float32x3'}`, stride 48 | **hand-copied** |

The Rust struct **is** the source of truth (it produces the bytes).
seer renders the native path, so a drift among {struct, native attrs,
WGSL} shows as a wrong lavapipe frame — but the **JS copy is unguarded**
(only a real browser exercises it). Proto/flatbuffers don't fit here:
they're for process-to-process wire formats, have no vocabulary for
`@location`/`format`/`stride`, and would put encode/decode on a
zero-copy hot path. The graphics-native fix is a single Rust
`INSTANCE_LAYOUT` const the native array is built from + a **contract
test that parses `main.ts` and asserts its offsets match** (keeps JS
hand-written per the no-wasm-bindgen axiom). Not built yet — see *Open
threads*; it's the prerequisite for branch wind.

### One WGSL layout, two vertex stages
`mesh_layout_wgsl!()` holds the shared ABI (Camera incl. `wind`,
VIn/IIn/VOut, `basis_from_axis`, light consts). MESH and LEAF each
`concat!` it with their OWN `@vertex` + `@fragment` — the mesh vs is
rigid, the leaf vs sways. The `mesh_and_leaf_shaders_share_one_layout`
test asserts both begin with the shared layout so the ABI can't drift
between pipelines even though their vertex stages differ.

### `TreeKind` is framework-free
Lives in `crates/cdda`, a thin tag — no bevy, no `&'static`. The game
owns `TreeKind → &'static TreeSpecies` (`species_for_kind`).
`tree_kind_tag` is **pinned** (Apple 0 … Dead 8); `stable_digest` +
`rotate_template` mix trees, so placement is stable under rotation and
across peers.

### The camera uniform now carries `wind`
`Camera { view_proj, wind: vec4 }`; `GpuCamera` is 80 bytes in both
backends. Non-mesh shaders declare only `view_proj` and ignore the
extra 16 bytes (a larger uniform buffer than a shader reads is valid).
`frame()` (web) / `render_scene` (native) write it; web sources seconds
from `FrameCount`, native passes a per-tour-stop phase.

---

## What's tested

- **cdda (32):** `every_shipped_building_resolves_deterministically`
  (resolve twice, require equal — replaced the golden-master hash
  snapshot; assert the invariant, don't pin the derived value),
  `orchard_resolves_to_a_grid_of_apple_trees`, `cell_to_tree`,
  digest/rotation over trees.
- **game (101):** `every_species_stays_in_unit_space_with_tips_in_the_crown`,
  `deadwood_is_a_per_tree_trait_bounded_by_species_odds` (odds-0 →
  none, OAK varies tree-to-tree, only tips die),
  `fungal_grows_witches_snot_where_apples_and_pines_grow_none`,
  `some_apple_trees_bear_fruit_and_pines_never_do`,
  `authored_apple_stands_a_little_taller_than_a_plain_authored_tree`,
  `trunk_fragment_paints_bark_from_the_uv`,
  `mesh_and_leaf_shaders_share_one_layout`,
  `wind_sways_leaves_but_not_trunks`, plus branch/species/tree_at_cell
  contract tests.

Not coverable by `cargo test`: that the GPU draws it. That's seer's
job — the frame PNG is the proof.

---

## Open threads (next session)

**The immediate next commit — instance ABI + branch wind (coupled):**
1. **Single source of truth for the instance layout.** One Rust
   `INSTANCE_LAYOUT` const; build the native attr array from it; a
   contract test parses `main.ts` and asserts offsets/stride/formats
   match. Closes the one unguarded copy (JS) without a codegen tool
   (honors no-wasm-bindgen). Optionally: screenshot the browser build
   in seer (Chromium is available) so the JS path gets a frame too.
2. **Branch wind.** Needs a per-instance sway weight — widen `i_axis`
   to `vec4` (`.xyz` orient, `.w` sway). Trunk sway 0 (rigid); branch
   sway ∝ thinness (`1 − base_radius/primary_radius`) so thinner limbs
   move more; the mesh vs pivots at each limb's base (`× v.pos.y`).
   This IS the ABI growth (1) exists to make safe — do 1 then 2.

**Sound:**
3. **Audible wind.** A low wind-rustle / leaf-and-branch rustle sample,
   volume-modulated, via the EXISTING `game_audio_play_samples` path
   (the campfire crackle already uses it) — no new `env.*` crossing.

**Render / material:**
4. **Walls-on-mesh** — the branch's *original* north star
   (`game/docs/RENDER.md`). Bark proved the UV carries a material; a
   SAMPLED texture image (bark/brick/damage) is the next step and needs
   a texture+sampler binding = the first real new `env.*` crossing.
   The doubled-edge wall-junction limitation is a mesh problem.
5. **Firelight + dimmer ambient.** No positional lights today — every
   fragment uses one fixed directional light; the campfire is a
   self-orange cube that lights nothing. Pass fire pos/intensity into
   the fragment + a night ambient so the fire's warm pool means
   something. Biggest payoff for "the world feels alive".

**Trees as data / taxonomy:**
6. **Species-aware authored height — DONE** (`authored_scale`); extend
   as more authored species land.
7. **Saplings.** CDDA `t_tree_young` / `*_young` currently map to
   `None` (no geometry) — `cell_to_tree` drops them. A sapling species
   (low `authored_scale`, few branches, `dead_limb_odds = 0`) would let
   them show. The deadwood/height machinery is already sapling-ready.
8. **Pears / lemons / nuts.** `cell_to_tree` folds the whole fruit/nut
   list onto `TreeKind::Apple`, so pears render *as* apples and citrus
   (`lemon`, unmapped) falls through to `Generic → OAK` with no fruit.
   Give distinct kinds their own species (pear yellow-green, citrus
   yellow). Botany note carried forward: pine "fruit" is a cone
   (gymnosperm — not a fruit), and walnut/pecan/hazelnut/chestnut are
   botanically fruits too, so the coarse "fruit/nut" bucket is fine as a
   *category* — it's appearance (colour, form) that should vary. CDDA
   itself has no taxonomy: flat `t_tree_<name>` ids, harvest behaviour
   attached per-terrain, which is why `cell_to_tree` pattern-matches
   substrings.

**Interaction / simulation (a whole missing system):**
9. Trees and fires are static render + AABB collider, not stateful
   interactables. This blocks: **shake** a tree (drop fruit — smallest),
   **chop** a tree (health + log drop), **harvest** the fungal snot /
   apples ("find a mushroom"), **tree-falls-on-building** (needs physics
   + damaging stamped props — largest), and **campfire burnout**
   (`Campfire { intensity }` has no fuel/lifetime — it flickers forever;
   add fuel decremented per tick, feeding as an interaction). All one
   theme: entities need mutable state + an action system.

**Housekeeping:**
10. **Open the PR.** Squash intent-sized; scrub any CC-BY-SA CDDA blobs
    before it's public. No PR exists yet (none requested).
