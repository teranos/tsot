# game/docs — render

The current shape of the game's render pipeline, and the mesh-based
successor that lands on the next branch.

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

This is the branch's **merge blocker**. The wall placement work in
this branch is functionally correct (see the tests in
`crates/cdda/src/placement.rs`) but the render doesn't cross the
"looks like walls" bar. Fix scoped to its own branch below.

## Next — mesh pipeline (own branch)

Replace / augment the cube-instance renderer with a **mesh
pipeline** so each building's wall system renders as one
continuous polyhedron.

**Concept.** Trace the wall boundary of each building as a 2D
polygon (outer perimeter + interior dividers), extrude to wall
height, cut door/window openings, generate a triangle mesh. Render
that as one draw call per building. Corners are where the polygon
turns 90° — no visible seam because there's no seam to render.

**Concrete work.**
1. New JS-side WebGPU import functions in `gpu_web.rs` — mesh
   pipeline creation, vertex-buffer-per-mesh, indexed draw with
   variable vertex count. Each new crossing is a hand-wired
   `env.*` import (see `imports.allow`) and needs mirroring into
   `seer-host`'s linker or the wasm fails to instantiate.
2. New shader for mesh rendering (parallel to `SHADER_WGSL`).
3. Mesh generation in the cdda crate — trace polygons from the
   wall lattice, tessellate with door/window cutouts. Or keep this
   game-side and have cdda emit a `WallGraph` the mesh generator
   consumes; either shape works.
4. Colliders — cdda's box `Prop.size` output stays; the mesh is
   for VISUAL. Player physics stays cube-based.
5. Ghost + glass passes need to consume the mesh too — the mesh
   generator can produce three variants (opaque / cut / ghost) or
   the same mesh renders in three passes with a per-pass mask.
6. Native `render.rs` mirror or accept native-differs-from-wasm.

**Estimated scope.** 10–20 hours of infra work before the first
correct-looking wall renders. Hence its own branch, not a squeeze
into this one.

**Deferred within this plan.**
- Per-face texture UVs / material variation. First cut: uniform
  wall material colour.
- Roof mesh. Slabs stay cube-instances until walls prove out.

## Frontier — render-adjacent

- **Player visibility (LOS from player)** — camera-frame cut-away
  today; the CDDA-style "you can only see what your character can
  see" is a whole visibility system (shadow-casting from player
  through the wall grid, fog-of-war state). Its own branch.
- **Multi-z-level render** — descend / ascend in an iso voxel
  world. Depends on cdda supporting multi-layer stamps.
