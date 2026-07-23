# game/docs — render

The current shape of the game's render pipeline, and the mesh-based
successor that lands next.

## Current pipeline — cube instancing

Every prop is a scaled cube (or thin box). Render passes:

- **opaque** — walls / roof / trees / obstacles / structures.
- **glass** — window panes, alpha-blended, depth-tested but not
  depth-writing.
- **ghost** — cut-away walls + roof at α=0.15 when the player is
  inside a roofed building.
- **UI** — HUD, watermark, dpad, bang overlay.

Every geometry is an instanced draw of one unit cube with per-
instance position, colour, and scale. Wall runs (see
[`../../crates/cdda/README.md`](../../crates/cdda/README.md)) are
already coalesced so adjacent cells emit ONE long prop — the seams
between run pieces are gone.

## Known limitation

Adjacent wall props render as **separate shaded boxes**. Even when
placement is exact (no overlap, no gap), the eye reads corners and
T-junctions as "two walls meeting" rather than "one wall turning."
Every wall junction — L corners, T-junctions, wall-meets-window,
divider-meets-perimeter — shows a doubled edge or subtle
discontinuity. No amount of placement-rule refinement inside the
cdda crate fixes this; the geometry is right, the rendering
substrate is wrong for reading as continuous.

This was the stamp-template merge blocker for the wall look. The
placement work is functionally correct (see the tests in
`crates/cdda/src/placement.rs`) but the render doesn't cross the
"looks like walls" bar. Fix scoped to its own scope below.

## Mesh substrate, proven on the tree — **shipped**

Proved the mesh substrate on the smallest thing: **the tree**. The
pipeline, the 52-byte `MeshInstance` layout, per-species baked
meshes and the browser parity all landed — see
[`TREES.md`](./TREES.md) for what shipped and what's still open on
the tree itself. The section below is kept as the original scope
statement.

**Intended behaviour once this lands.**

1. A second render pipeline exists alongside the cube-instance
   pipeline. Both run each frame. The cube pipeline draws
   everything it draws today except trees.
2. Trees render through the mesh pipeline:
   - Trunk = tapered geometry (truncated cone, ~12 sides), shared
     mesh, instanced per tree with per-instance transform.
   - Canopy = **phyllotactic** — elements placed at successive
     golden-angle rotations (≈137.5°) around the growth axis,
     radius scaling per step. Reads as a real crown, not a sphere
     or a box.
3. Placement stays deterministic per world cell (same inputs as
   `trees::tree_at_cell`), so streaming and reproducibility are
   unchanged.
4. seer captures `[perf]` for both pipelines in the same frame —
   first real numbers on mesh cost vs cube cost.
5. Vertex format reserves UV slots from day one, even though the
   first cut has no textures. Damage textures (cracks, scorches)
   are a real downstream goal; skipping UVs now costs a second
   swap later.

**Out of scope here.** Walls, buildings, obstacles, trails, HUD,
player, glass, ghost — all still on cubes. No destruction. No
debris. No textures. No collider changes.

**Merge criterion.** Trees look like trees. Nothing else regresses.
seer's frame captures confirm both.

## Current scope — walls on mesh

Replace the cube-instance walls with a **mesh** per building so
each wall system renders as one continuous polyhedron. Corners are
where the geometry turns — no seam because there's no seam to
render.

### The goal (the merge bar)

> The lavapipe tour render shows the school's and houses' wall
> corners reading as **one wall turning**, not two boxes meeting —
> compared directly against the cube-wall baseline frames captured
> before this scope started
> ([`img/walls-cubes-baseline-school.png`](./img/walls-cubes-baseline-school.png),
> [`img/walls-cubes-baseline-house.png`](./img/walls-cubes-baseline-house.png)).
> Nothing else in the frame regresses; seer's `[perf]` captures
> mesh cost vs the cube walls it replaces. Same bar shape as
> TERRAIN.md: development is validated by looking at the render,
> under strict TDD (failing test first) and errors-surfaced.

### Locked decisions

1. **cdda emits a `WallGraph`; game tessellates.** `Template`
   grows a third layer exactly as it grew `trees`: **nodes are
   wall-line cells** (the CDDA damage quantum — CDDA bashes per
   tile), each carrying its kind (`Solid`, `Window`, `Door`) and
   material colour; **edges are their 4-adjacencies**. Runs, closed
   loops (enclosure), junction degrees, miters and the mesh all
   derive from this primitive. The graph is classified by the same
   `cell_wall_kind` that backs `is_wall_line_char`, so the graph
   and the flood-fill can never disagree about what seals a room —
   and it is never traced back out of coalesced `Prop` boxes. The
   `Prop` path stays untouched: it remains the collider source and
   the render fallback. The graph rotates in `rotate_template` and
   mixes into `stable_digest`, both trees-style. Window sill and
   lintel heights stay tessellation constants, not authored data.
2. **Stable edge identity, from day one.** Edge IDs are
   deterministic (derived from the authored lattice, asserted by
   test). Reason: breaking a wall, burning it, curtains, broken
   windows are hard requirements of the game. None of that ships
   in this scope — but mutable wall state needs an *address*, and
   the edge is the CDDA-idiom damage quantum (CDDA bashes per
   tile). This decision is the entire extent to which dynamism is
   in scope here: the address space exists, no mutation does.
3. **Rectilinear tessellation — no general tessellator.** Every
   wall is axis-aligned and every opening is an axis-aligned
   rectangle at known heights (the sill/glass/lintel stack already
   models this). A wall face is horizontal quad bands — solid
   below the sill, open at the glass, solid above the lintel —
   never a polygon-with-holes. Jamb faces at door/window openings
   are correct (visible wall thickness); faces *inside* joints are
   the bug this scope exists to kill.
4. **Nodes own the miter.** Corner and T-junction plan geometry is
   resolved in the node's thickness×thickness square, once, at
   tessellation. The 24-unit junction-shortening in `placement.rs`
   (a cube-volume z-fight workaround) becomes render-dead; it
   stays in the prop path only because colliders still read it.
5. **Weld positions, not normals.** A 90° corner keeps a hard
   crease with two normals — that is what a real wall turning
   looks like. The doubled-edge artifact comes from two separate
   boxes shading their own end-caps inside the joint, not from the
   crease.
6. **Bake per building, cached and re-bakeable.** One mesh per
   building, generated lazily, cached on a key that *can*
   invalidate — `(building_index, version)`, version constant in
   this scope. The terrain surface set the precedent (cached on a
   snap key, regenerated on change); when wall state becomes
   mutable, mutation bumps the version and the next frame re-bakes
   — the render architecture does not change again. One identity
   `MeshInstance` per building at its pad anchor; the
   `scene::drape` choke point is unchanged.
7. **No new `env.*` crossings, no new shader.** The terrain
   surface already proved runtime mesh creation reuses the
   existing mesh crossing — `imports.allow` unchanged. Walls draw
   through the existing mesh WGSL with uniform material colour.
   UVs are planar per run (u = distance along the wall,
   v = height) so damage/crack shading lands later with zero
   format changes — the reserved UV slots exist for exactly this.
8. **Cut-away = two index ranges.** The iso camera direction is
   fixed (eye is always player + (d,d,d)), so which faces are
   camera-facing is a **static property of the bake**: classify
   every face by outward normal (`n.x + n.z > 0` → near) and emit
   near/far index ranges over one vertex buffer. Outside: both
   opaque. Inside this building: far opaque, near + roof in the
   ghost pass. No per-frame regeneration, no clip planes. A code
   comment records the for-now assumption: a rotating camera
   breaks this bake.
9. **Glass and roof stay where they are.** Window panes remain
   cube instances in the alpha-blended glass pass — the wall mesh
   leaves the hole they sit in. Roof slabs stay cube instances
   until walls prove out.
10. **Colliders unchanged.** cdda's box `Prop.size` output is
    still what physics consumes; the mesh is visual.

### TDD slices

Each slice is failing-test-first; nothing else may regress.

1. **`WallGraph` in cdda** — on the existing synthetic fixtures
   (the P-shape building test): the perimeter forms closed loops
   (cyclomatic number pins the room count), a T-junction node has
   degree 3, door and window nodes carry their kinds, and node/edge
   IDs are identical across two loads. Pure data; no render.
   **Landed** — `template.rs` (`WallGraph`/`WallNode`/`WallEdge`/
   `WallCellKind`, digest + rotation covered), `placement.rs`
   (row-major build), `cells.rs` (`cell_wall_kind`); diagnostics
   via `cargo run --example wall_graph_dump` (P-shape or a corpus
   house).
2. **`wall_mesh.rs`: straight run + corner** — the corner test is
   the point of the whole scope: the mesh is manifold along the
   joint (every interior edge shared by exactly two triangles) and
   emits **zero faces inside the node's miter square**. That is
   the machine-checkable proxy for "one wall turning."
   **Landed** — `game/src/wall_mesh.rs`: runs are quad bands split
   around junction squares, squares emit only unattached faces,
   jambs cap runs at door cells, manifold-or-grounded pinned by
   test on the straight / L / T fixtures AND the full P-shape
   graph end-to-end. Slice 3's opening bands and the door-gap
   invariant are partly pre-paid (doors already tessellate as
   gaps with jambs). Diagnostics:
   `cargo run --example wall_mesh_preview [house_01]`. Known
   follow-up for slice 4: `MeshVertex` has no colour channel —
   per-material wall colour needs per-vertex colour, a colour →
   submesh split, or one instance per colour group at bake.
   The colour design target (from discussion): walls carry their
   exterior material colour from the graph, and INTERIOR faces get
   per-room colours — rooms derived from the graph's enclosure
   faces, palette seeded per building (HOUSE_VARIANTS-style) so
   colours vary across buildings but cohere within one. A wall
   face's colour then depends on which side of the wall it is and
   which room that side bounds — one more reason the colour
   channel wants to live per-vertex (the two side faces of one
   band differ), decided at bake.
3. **Openings** — no wall triangle intrudes into a window's glass
   band; doorways get jambs; band faces of one run lie only in the
   run's two side planes.
   **Landed** — universal Y-banding at the sill/lintel lines (every
   vertical face splits on `Y_BANDS`, so solid-meets-window stays
   manifold by construction); windows emit sill + lintel + reveals,
   glass band open for the alpha pass; door jambs came free with
   slice 2. Pinned by the window fixture (glass-band-open,
   coplanarity, manifold) and the same assertions over every
   window node of the P-shape graph end-to-end.
4. **Bake + draw swap** — buildings with a graph stop emitting
   `Wall*` (and window sill/lintel) cube instances; glass panes
   keep emitting. The lavapipe tour render is the merge bar,
   against the baseline frames. seer `[perf]` captures the cost.
   **Landed** — `wall_bake.rs`: per-building bakes from the
   rotated graph (chunk-scan, pure), colour-grouped parts, and the
   ROOM COLOUR model from design discussion arrived early:
   interior faces colour per room (graph-derived flood fill),
   palette seeded per building, exterior keeps material. Cut-away
   went one better than the static split: near triangles are
   depth-SORTED at bake, so the draw range cuts exactly at the
   player's depth (cube-path parity) with zero re-uploads. Both
   render paths draw the parts (native `render_scene`, browser
   `frame_walls` with a chunk-crossing bake cache) — slice 6 is
   mostly pre-paid; its deployed-pixel check remains. Amendment
   to decision 7: walls DID need their own WGSL (the mesh shader
   bakes procedural BARK into everything it draws) — same layout,
   same pipeline factory, plain Lambert with higher ambient for
   vertical faces; still zero new `env.*` crossings
   (`imports.allow` unchanged). Two lessons the render taught:
   quad winding was CW-from-normal (every wall drew inside-out,
   ambient-dark — invisible to the manifold tests, caught by the
   first GPU frame), and near/far must classify by POSITION, not
   face normal (a far wall's interior face points at the camera
   but is the backdrop). Known cosmetic follow-up: a top-cap
   sliver can show where the depth cut crosses a wall run.
5. **Near/far ranges + ghost parity** — the `emit.rs` property
   (ghost = exactly what opaque skipped, never both) holds for the
   mesh's index ranges.
   **Landed** — the invariant holds by construction: the bake
   stores the near triangles twice, depth-ASCENDING for the opaque
   draw (visible prefix grows with player depth) and
   depth-DESCENDING in `ghost_indices` (the cut set is always a
   prefix — the web mesh crossing can only draw prefixes). The
   complement property is pinned for every possible cut depth by
   test. `WALL_GHOST_SHADER_WGSL` draws the cut set at α=0.15,
   alpha-blended, depth-tested, not depth-writing, after glass, on
   BOTH paths (the native path gains its first ghost pass). The
   `create_mesh` factory crossing grew a `ghost` flag parameter —
   same import set, `imports.allow` unchanged, JS shim + seer-host
   mirror updated in the same diff.
6. **Browser parity** — `seer-imports-check` green with
   `imports.allow` unchanged (decision 7 says no new crossings; a
   test proves it); `web_shim` ABI tests hold; merge bar is the
   deployed pixel, per TERRAIN.md Slice 8 precedent.

### Deferred (recorded, not this scope)

- **Wall mutation overlay** — per-edge state
  (`Damaged`/`Breached`, curtains open/closed/broken glass, fire
  as a per-edge process driving charring → breach). The overlay is
  runtime world state layered over the immutable authored graph;
  render bake **and collider emission** must both consume it (one
  source of truth, or a hole you can see through still blocks).
  Contested canonical state, flower-pickup-style, when it lands.
- Damage/crack textures riding the reserved UVs.
- Reading CDDA's own bash data (`str_min`, what a wall becomes)
  instead of authoring our own — cdda README frontier question 2.
- Per-face material variation; roof mesh.
- **Enclosure ("room") as a derived artifact of the `WallGraph`.**
  The graph's closed loops define enclosed space in two strengths:
  the *hull* (what the perimeter seals — fences deliberately don't,
  per the flood-fill rule) and the *faces* (individual rooms).
  Consumers queue up beyond render: terrain's flatten rule could
  shrink from the full stamp rectangle to the hull + apron so the
  yard rolls with relief (TERRAIN.md decision 5 left "whether the
  yard ever rolls" open — the hull is the data that would answer
  it; yard props must then drape per-prop, and authored yard
  content bounds how much roll it tolerates); the roof cut-away's
  radius heuristic becomes point-in-hull; LOS, sound propagation,
  fire spread and room labels all read the same structure. None of
  this is walls-on-mesh scope — but the graph is specified with
  closed loops precisely so these consumers can exist.

## Frontier — render-adjacent

- **Player visibility (LOS from player)** — camera-frame cut-away
  today; the CDDA-style "you can only see what your character can
  see" is a whole visibility system (shadow-casting from player
  through the wall grid, fog-of-war state). Own scope.
- **Multi-z-level render** — descend / ascend in an iso voxel
  world. Depends on cdda supporting multi-layer stamps.
