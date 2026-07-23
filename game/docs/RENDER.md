# game/docs — render

The game's render pipeline: what draws how, and the record of the
two mesh scopes (trees, walls) that replaced cube instancing where
it mattered.

## The passes, today

- **opaque cubes** — roofs, furniture, obstacles, player, NPCs,
  trails, pins. One unit-cube vertex buffer, per-instance
  position/colour/scale.
- **mesh** — the solid terrain surface (ground shader: Lambert +
  the world-anchored reference grid), building walls (wall
  shader: plain Lambert, per-room interior colours), tree wood
  (mesh shader: procedural bark) and canopy (leaf shader:
  silhouette cards). Indexed instanced draws, one WGSL layout
  (`MeshVertex` 0/1/2 + `MeshInstance` 3/4/5/6, held to
  `INSTANCE_ATTRS`).
- **glass** — window panes, alpha-blended, depth-tested, not
  depth-writing.
- **ghost** — cut-away geometry at α=0.15 when the player is
  inside a roofed building: the roof (cube ghost) and the cut
  near walls (wall-mesh ghost). Ghost = exactly what the opaque
  draw skipped, by construction.
- **UI** — HUD, watermark, dpad, bang overlay.

Both render paths (native lavapipe → PNG for seer, browser via the
hand-wired `gpu_web` seam) draw the same passes from the same
shared emit/bake code.

## Mesh substrate, proven on the tree — shipped

The mesh pipeline was proven on the smallest thing first: the
tree. Trunk/branch isosurface meshes per species, phyllotactic
canopy, day-one UV slots, browser parity — see
[`TREES.md`](./TREES.md) for the full record and the open tree
work. The bar was "trees look like trees; nothing else regresses."

## Walls on mesh — shipped

Each building's wall system renders as one continuous polyhedron
baked from its `WallGraph`. Corners are where the geometry turns —
no seam because there is no seam to render. This closed the
long-standing cube-wall limitation: adjacent wall props used to
render as separate shaded boxes, so every junction showed stub
pillars, T-junction gaps, or doubled edges no placement rule could
fix.

**Merge bar — met.** The lavapipe tour renders against the cube
baselines
([`img/walls-cubes-baseline-school.png`](./img/walls-cubes-baseline-school.png),
[`img/walls-cubes-baseline-house.png`](./img/walls-cubes-baseline-house.png))
show corners reading as one wall turning; nothing else regressed;
the deployed browser build was verified on-device (TERRAIN.md
Slice 8 precedent). Developed failing-test-first throughout; cdda
38 / game 144 tests green, `imports.allow` byte-identical.

### What shipped

- **`WallGraph`** (cdda) — the wall-line topology as a third
  `Template` layer, trees-style. Nodes are wall-line cells (the
  CDDA damage quantum) carrying kind (`Solid`/`Window`/`Door`),
  material colour, and per-axis lateral centerline offsets from
  the same slot classification the prop path emits from — mesh
  walls sit exactly where collider walls sit. Edges are
  4-adjacencies. Node/edge ids are deterministic (row-major):
  the stable address future wall mutation writes to. Rotates in
  `rotate_template` (ids survive), mixed into `stable_digest`.
  One cell classifier (`cell_wall_kind`) backs both the graph and
  the flood-fill, so they can never disagree about what seals a
  room. Diagnostics: `cargo run --example wall_graph_dump`.
- **`wall_mesh.rs`** (game) — rectilinear tessellation, no general
  tessellator. Runs are quad bands split around junction miter
  squares (nodes own the miter; squares emit only faces no run
  abuts). Every vertical face splits on the canonical sill/lintel
  lines (`Y_BANDS`), so solid-meets-window stays manifold by
  construction: windows occupy the outer bands, the glass band
  stays open for the alpha pass, reveals close the solid
  neighbours, and a run capping at a door cell IS the jamb.
  Positions weld, normals crease (a 90° corner keeps its hard
  edge — the artifact was end-caps inside joints, never the
  crease). Invariants pinned by test: manifold-or-grounded on
  fixtures AND the full P-shape graph, zero faces inside any
  miter square, glass bands open, side faces coplanar with the
  run planes. Diagnostics:
  `cargo run --example wall_mesh_preview [house_01]`.
- **`wall_bake.rs`** (game) — per-building bakes from the rotated
  graph. Colour-grouped parts via a per-face resolver: exterior
  faces keep the authored material; **interior faces colour per
  room** — rooms derived by flood fill over the graph's cells,
  palette seeded per building (`ROOM_PALETTE`, one array — tune
  there), so buildings vary and rooms within one cohere. Cut-away
  is data, not passes: near (camera-side) triangles are stored
  depth-ASCENDING after `near_start` for the opaque draw and
  depth-DESCENDING in `ghost_indices` for the ghost draw, so both
  per-frame ranges are prefix counts (`WallPart::draw_counts`) —
  the cut stops exactly at the player's depth, the ghost is
  exactly the skipped set, zero per-frame re-uploads. FOR NOW:
  a rotating camera would invalidate this bake.
- **Both paths** — native `render_scene` takes `WallDrawPart`s;
  browser `frame_walls`/`frame_walls_ghost` re-bake + re-upload
  only on player chunk crossings. The `create_mesh` factory
  crossing gained a `ghost` flag (same import set; JS shim +
  seer-host mirror updated together). Walls draw with their own
  WGSL — `WALL_SHADER_WGSL` (plain Lambert, higher ambient for
  vertical faces) and `WALL_GHOST_SHADER_WGSL` — because the tree
  mesh shader bakes procedural bark into everything it draws; all
  three share `mesh_standard_vs_wgsl!` and the layout macro.
- **emit.rs contract** — `Wall*` and window sill/lintel props
  never become cube instances (they remain the collider + bake
  source); glass panes keep emitting; the cube ghost carries only
  the roof.

### Locked decisions that held

1. cdda emits the graph; game tessellates — never traced back out
   of coalesced `Prop` boxes. The `Prop` path is untouched:
   collider source and render fallback.
2. Stable per-cell identity from day one — the address space for
   wall mutation (break/burn/curtains), which ships later as an
   overlay; the entire extent of dynamism in this scope.
3. Rectilinear only; openings are axis-aligned bands; sill/lintel
   are tessellation constants, not authored data.
4. Colliders unchanged; the mesh is visual.
5. Glass panes and roof slabs stay cube instances.
6. Bakes are cached and re-bakeable — when wall mutation lands,
   a version bump re-bakes; the render architecture is done
   changing.

### Lessons (paid for once)

- `f32::to_bits` sorting reverses negatives — split a run at the
  template centerline and re-created the doubled-corner artifact
  the scope exists to kill. `total_cmp`.
- Quad winding was CW-from-normal: every wall drew inside-out and
  ambient-dark. Invisible to manifold tests; caught by the first
  GPU frame. The render is the validator.
- Cut-away classification must use POSITION, not face normal — a
  far wall's interior face points at the camera but is the
  backdrop you must keep.
- The 24-unit junction-shortening in `placement.rs` is render-dead
  (it was a cube z-fight workaround); it stays only because
  colliders read the props.

### Known follow-ups (surfaced, not silent)

- A top-cap sliver can show where the depth cut crosses a wall
  run (cap quad centroid straddles the threshold its side faces
  don't). Cosmetic.
- Startup template load grew (~76→195 ms one-time worst frame in
  debug tours): every template/variant now also builds its graph
  + offsets at load. One-time cost; cache or lazy-build if it
  ever matters.
- Room palette is deliberately muted; louder is a one-array edit
  in `wall_bake.rs`.

### Deferred (recorded, not this scope)

- **Wall mutation overlay** — per-cell state
  (`Damaged`/`Breached`, curtains open/closed/broken glass, fire
  as a per-cell process driving charring → breach) layered over
  the immutable authored graph. Render bake AND collider emission
  must both consume it (one source of truth, or a hole you can
  see through still blocks). Contested canonical state,
  flower-pickup-style, when it lands.
- Damage/crack textures riding the reserved UVs; reading CDDA's
  own bash data (cdda README frontier question 2).
- Roof mesh; per-face material variation.
- **Enclosure ("room") as a derived artifact of the `WallGraph`.**
  Closed loops define enclosed space in two strengths: the *hull*
  (what the perimeter seals — fences deliberately don't) and the
  *faces* (individual rooms — already consumed by room colours).
  Queued consumers: terrain's flatten rule shrinking from the
  stamp rectangle to hull + apron so yards roll (TERRAIN.md
  decision 5's open call); point-in-hull replacing the roof
  cut-away radius heuristic; LOS, sound propagation, fire spread,
  room labels.

## Frontier — render-adjacent

- **Player visibility (LOS from player)** — camera-frame cut-away
  today; the CDDA-style "you can only see what your character can
  see" is a whole visibility system (shadow-casting from player
  through the wall grid, fog-of-war state). Own scope.
- **Multi-z-level render** — descend / ascend in an iso voxel
  world. Depends on cdda supporting multi-layer stamps.
