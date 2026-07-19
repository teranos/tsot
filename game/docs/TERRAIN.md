# game/docs — terrain

Terrain height for the game, in the SimCity 4 idiom, validated by
the draped dev-grid on the headless lavapipe render.

## The goal (the merge bar)

> The game renders SC4-style terrain height through the mesh pipeline
> (cube pipeline not reused), where every CDDA mapgen stamp sits on an
> entirely flat pad and all elevation change occurs outside those pads,
> and the retained dev-grid, draped over the heightfield, is the
> validation instrument — with the headless lavapipe renders visibly
> demonstrating flat stamp bases and relief only outside them. That
> render is the merge bar; development is validated by looking at it
> (plus seer's `[perf]` for cost), under the repo's strict-TDD rule
> (failing test first) and errors-surfaced rule.

Precedent: `RENDER.md` proved the mesh substrate on the smallest thing
(the tree) with the bar "trees look like trees." This is the same
move — prove the heightfield substrate on the smallest thing (the
draped grid) with the bar "flat pads under stamps, relief only
outside."

## Scope now — draped grid only

The single deliverable of this branch is the **heightfield + flat
stamp pads + the draped dev-grid that proves them**, seen in the
render. Nothing more.

**Non-goals (explicitly out, now):**

- No oceans, lakes, rivers — no water of any kind.
- No mountains, cliffs — relief stays gentle; no dramatic landforms.
- No clay, sand, beaches — no materials, no terrain textures, no
  slope/altitude surface shading.
- No solid shaded terrain surface mesh yet — the draped grid is the
  instrument; the filled surface comes later, off this branch.
- No iso-voxel / multi-z-level descent (`RENDER.md` frontier) — that
  is a separate, layered concern, not the terrain model here.
- No retiring the cube pipeline for existing props — it stays for
  player / obstacles / walls / structures; it is simply **not
  extended** into terrain-era geometry.

## Locked decisions

1. **Target look is SimCity 4** — a continuous, single-valued
   heightfield `height(x, z)`. Not iso-voxel, not marching-cubes
   terrain.
2. **Mesh pipeline only.** Terrain geometry and the draped grid are
   mesh-pipeline citizens (the `MeshVertex` / `MeshInstance` path the
   trees already use, `tree_mesh.rs:16` / `scene.rs:106`). The cube
   pipeline is not reused for either.
3. **The dev-grid is never removed.** It is the development
   validation instrument. It drapes onto `height(x, z)` (each vertex
   sampled) and is the primary read-out for relief, seams, slope, and
   — above all — flatness inside stamps.
4. **CDDA stamps are respected in their entirety — non-negotiable.**
   Every stamp footprint is a flat pad at a single pad height.
   Elevation change happens **only outside** the flat area. This is
   baked into `height(x, z)` itself (one source of truth), so terrain
   mesh, draped grid, props, and player all agree by construction.
   The reason is structural, not cosmetic: CDDA mapgen is authored on
   a flat single-z grid, so every tile in a stamp assumes coplanar
   neighbours; any relief intruding into a footprint would tear the
   authored layout.
5. **Flat area = the full authored stamp footprint, including the
   yard** (the extent `half_extents` describes), for now. When the
   flatten path is implemented, record this as a code comment marking
   it a for-now choice — do **not** document how it might change;
   whether the yard ever rolls is the implementer's call.
6. **Validation is the render.** `game-native` → PNG on lavapipe
   (llvmpipe, Vulkan software) is the channel. The tour already
   visits the school and a house, so the flat-pad bar is checkable in
   the PNG. seer's `[perf]` covers the added mesh cost.

## Where it plugs into the current code

The substrate today is a hard flat plane at y=0; the pieces the work
touches:

- **Flat floor** — a single scaled cube instance, top surface at y=0
  (`scene.rs:355`). The heightfield is what eventually replaces it;
  for the draped-grid milestone it may stay beneath the grid (the
  grid is the instrument regardless).
- **Dev-grid** — thin-cube lattice at y=0.1, `GRID_HALF=2000`,
  `CELL_STEP=80` (CDDA cell size), `scene.rs:373-413`. This is what
  drapes; it stops being cube instances.
- **CDDA placement** — `building_anchor_in_chunk`, `building_index`,
  and template `half_extents` (the deterministic queries
  `seer_tour_from` uses, `lib.rs:178`; `chunk.rs:93`) give every
  stamp's position and extent. The flatten test reads them.
- **Camera** — orthographic true-isometric, already follows
  `player.y` (`scene.rs:52-62`). SC4's projection; no change needed
  for relief to be visible.
- **Render path** — `render::render_scene` → 512×512 PNG
  (`render.rs`), driven by `SEER_FRAME_PATH` / `SEER_MULTI_FRAME_DIR`
  (`lib.rs render_single` / `render_snapshots`).

Determinism is preserved: `height(x, z)` is pure, over the same
per-cell integer coordinate model as `tree_at_cell`.

## TDD slices

Each slice is failing-test-first. Slices state the invariant the test
pins and what "done" means; the mechanics (noise choice, blend shape,
line topology) are the implementer's. Nothing else may regress.

### Slice 1 — `height(x, z)` exists, deterministic, continuous
- **Test:** `height` is pure (same input → same output); C0-continuous
  across a chunk seam (adjacent samples, including across a chunk
  boundary, differ by a bounded amount); base relief shows real
  variation but bounded amplitude and slope (no cliffs/mountains).
- **Done:** a base relief function exists and passes; the world is
  still otherwise unchanged.

### Slice 2 — flat pads inside CDDA stamps (the non-negotiable)
- **Test:** for the school stamp the tour locates (largest footprint,
  spans multiple chunks), **every** sampled point inside its full
  authored footprint — including yard — returns one identical constant
  pad height; points outside vary. The flatten lookup considers every
  stamp whose footprint could cover a point, so the multi-chunk school
  is covered in its entirety (search radius ≥ the max half-extent).
- **Done:** flattening is baked into `height(x, z)`; the school test is
  green. A code comment on the flatten path records decision #5 (full
  footprint incl. yard, for-now) with no how-to.

### Slice 3 — transition skirt outside pads
- **Test:** `height` is continuous from every pad edge out to the
  surrounding relief (no discontinuity, no unbounded step at the
  footprint boundary); the pad interior stays exactly flat (Slice 2's
  invariant still holds after the skirt exists). Elevation change
  occurs only outside the flat area.
- **Done:** pads reconnect to relief via a skirt; both invariants
  green.

### Slice 4 — draped dev-grid as mesh geometry (cube pipeline not used)
- **Test:** dev-grid vertices are emitted through the mesh pipeline at
  `height(x, z)` + ε (grid vertex Y equals the sampled height plus the
  lift); no cube instances are emitted for the grid.
- **Done:** the thin-cube grid (`scene.rs:373-413`) is replaced by
  draped mesh/line geometry; it traces the relief; the cube pipeline is
  untouched and unextended.

### Slice 5 — render shows it (the merge bar)
- **Test:** render the tour on lavapipe (`game-native`,
  `SEER_MULTI_FRAME_DIR`); over the school and house footprints the
  draped-grid samples are coplanar (machine-checkable proxy for "flat
  pad"), and vary outside. Human check: the PNG shows the grid flat
  over stamp bases and warped only outside them. seer `[perf]` records
  the added cost. Nothing else in the frame regresses.
- **Done:** the merge-bar render exists and reads correctly.

### Landed after Slice 5 (render-draping + amplitude)

The whole scene now drapes onto the terrain at render: buildings sit on
their (flat) pads, trees/player/props follow the surface, via one choke
point — `scene::drape` / `drape_mesh` (a new entity type drapes for free
if it lands in an instance stream). The flat backdrop floor was removed.
Amplitude 140 → 300 (140 was too shallow to read in the render). This is
render-time only; the sim is still flat (that's Slice 7).

### Slice 6 — solid colored terrain surface (the ground itself)
- **Test:** a per-chunk heightfield surface mesh — `MeshVertex` grid,
  two triangles per cell — has vertices on `height(x, z)` and
  unit-length normals derived from the heightfield gradient; the mesh is
  coplanar with up-normals over the school footprint and varies outside;
  emitted through the mesh pipeline (never cubes).
- **Done:** on lavapipe the ground reads as a solid, Lambert-lit surface
  — lit hills, shaded valleys, flat pads under buildings — not a
  wireframe over black. ONE ground colour from geometry (not grass/rock
  biome materials — those stay deferred). Answers "why no colour." The
  dev-grid stays, draped just above it as the validation overlay.

### Slice 7 — player & physics walk the terrain (height is real, not render-only)
- **Test:** advancing the player across a slope sets its `Position.y` to
  `height(x, z)` at the new XZ (deterministic); walking onto a stamp
  footprint puts it at the pad height. Sim-driven — not the render-time
  lift.
- **Done:** the player (and NPCs) move along the surface in the
  simulation; the render-time drape and follow-camera derive from the
  real position, with NO double-lift for sim-driven entities.

### Slice 8 — browser / wasm parity (make it real at game.sbvh.nl)
- **Test:** `seer-imports-check` passes with every new wasm↔browser
  crossing declared in `imports.allow` (the no-wasm-bindgen boundary);
  `web_shim_*_layout_matches_this_const`-style tests hold the JS
  descriptors to the Rust ABI.
- **Done:** the browser render (`render_web` + `gpu_web` + JS shim) draws
  the surface, draped grid and draped world like native. Merge bar is a
  **browser** render — a headless-Chromium (Playwright) screenshot of the
  web bundle, or game.sbvh.nl — showing non-flat terrain.

## Checklist

- [x] Slice 1 — `height(x, z)` pure, deterministic, C0-continuous
      across chunk seams; gentle bounded relief
- [x] Slice 2 — every point in the school's full authored footprint
      (incl. yard) returns one identical pad height; outside varies
- [x] Slice 2 — flatten lookup radius ≥ max stamp half-extent (the
      multi-chunk school is covered in its entirety)
- [x] Slice 2 — code comment records "flat area = full authored
      footprint incl. yard, for now" (no how-to)
- [x] Slice 3 — skirt makes `height` continuous from pad edge to
      relief; pad interior stays exactly flat
- [x] Slice 4 — dev-grid emitted through the mesh pipeline, draped at
      `height(x, z)` + ε
- [x] Slice 4 — no cube instances emitted for the grid; cube pipeline
      unextended (removal landed with Slice 5's render swap)
- [x] Slice 5 — lavapipe render: grid draped in the game frame, flat
      over stamp pads, rolling over open relief
- [x] Slice 5 — seer `[perf]` captures the added mesh cost
- [x] Merge bar — the render visibly demonstrates flat stamp bases and
      relief only outside them; nothing else regresses
- [x] Post-5 — whole scene render-draped via one choke point
      (`scene::drape`); backdrop floor removed; amplitude 300
- [ ] Slice 6 — solid heightfield surface mesh: verts on `height`,
      gradient normals, coplanar over pads
- [ ] Slice 6 — renders as solid Lambert-lit ground on lavapipe (lit
      hills, shaded valleys, flat pads); one ground colour
- [ ] Slice 7 — player `Position.y` follows `height(x, z)` in the SIM;
      pad height on a footprint; no render double-lift
- [ ] Slice 8 — `imports.allow` + boundary check green with the new
      wasm crossings; JS shim layout tests hold
- [ ] Slice 8 — browser render (headless Chromium / game.sbvh.nl) shows
      non-flat terrain like native

### Known follow-ups (surfaced, not silent)

- **Browser parity is Slice 8.** Until then the wasm/browser render
  mirrors none of this (grid, surface, draping) — the cube grid was
  removed from the shared instance path, so game.sbvh.nl has no terrain.
- Draping is **render-time only** — the sim is still flat XZ. Slice 7
  makes the player's height real; guard against double-lifting
  sim-driven entities that then carry a real `y`.

## Deferred (not this branch)

Recorded so the boundary is explicit; no slices here. (The solid
surface, the world-draping, and terrain movement graduated to Slices
6/7 and the post-5 work; what stays deferred:)

- Slope/altitude **materials** (grass / rock / snow), textures, UVs —
  Slice 6 is one lit ground colour, not biome materials.
- Water plane at sea level; oceans, lakes, rivers.
- Dramatic landforms — mountains, cliffs, beaches, sand, clay.
- Iso-voxel multi-z descent/ascent (`RENDER.md` frontier).
