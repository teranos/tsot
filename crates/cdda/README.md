# cdda

The seam between Cataclysm: Dark Days Ahead's authored mapgen JSON
and our world. Given a pinned CDDA release + a manifest of files to
embed, this crate produces `Template` values a consumer can stamp
into an ECS. The crate is framework-agnostic (no Bevy, no render,
no obs) — game does the stamping and observability routing on its
side. See [`../../ERROR.md`](../../ERROR.md) for the sacred-error
axiom the failure-return convention follows.

## What ships

`load_building_templates() -> (BuildingTemplates, Vec<String>)`
resolves every shipped mapgen once, in a canonical order:

- **`garage`** — a small one-tile CDDA garage.
- **`shed`** — hand-authored (`assets/buildings/shed.json`) because
  CDDA has no standalone shed; its sheds are `place_nested` pieces.
  Deletable once `place_nested` support lands.
- **`daycare`** — a single-tile civic building, walls + windows
  inline (no palette dep beyond the roof palette we already fetch).
- **`school`** — a 3×3 (72×72) **multi-tile** building, driven by
  its inline `school_palette`. Streams via game's grid-sliced
  per-chunk path in `chunk.rs`.
- **`house_01..04`** — palette-driven layouts, each rolled through
  `HOUSE_VARIANTS` (=6) palette seeds → 24 seeded houses.

Failure strings are returned alongside the templates (sacred —
never dropped); the game routes them to obs.

## Placement model

Wall emission is **run-based**, not per-cell. Contiguous same-slot
cells (e.g. a top-perimeter row) coalesce into one long `WallEW` /
`WallNS` prop so there are no seams between adjacent pieces at the
placement level. Windows expand into three stacked layers
(sill + glass + lintel) using the neighbouring wall's material
colour. Doors are gaps in the run — no wall prop, so runs on either
side stop naturally.

Junctions currently shorten NS runs by 24 units at their EW-facing
ends so perpendicular walls don't share a 3D volume (z-fight
avoidance). See the **known rendering limitation** below.

## Rendering (walls-on-mesh — resolved)

Walls no longer render from the `Prop` boxes: the crate emits a
`WallGraph` (a third `Template` layer — wall-line cells with kind,
material colour and lateral centerline offsets, plus their
4-adjacencies) and the game tessellates each building into one
continuous mesh. The `Prop` path remains the collider source and
render fallback; the junction-shortening below survives only for
colliders. See [`../../game/docs/RENDER.md`](../../game/docs/RENDER.md)
for the shipped scope.

## Deferred (not this crate yet)

- **`place_nested`** — the #1 unhandled feature in the coverage
  report (`cargo run --bin cdda-coverage`, 654 hits at the time of
  the last measurement). Lets CDDA's nested pieces (sheds, small
  extras) drop into buildings, and lets `shed.json` be deleted.
- **Furniture beyond toilets.** Chairs / tables / beds are dropped
  from the CDDA furniture map today because game has no pickup or
  interaction mechanic — spawning them would just block movement
  without letting the player take them. A specific carve-out
  spawns `f_toilet` as a `PropKind::Toilet` (small ceramic box);
  everything else stays disabled until game gains interaction.
- **Door state.** Doors are gaps in wall runs on this branch — no
  `open`/`closed` state, no prop. When state matters (e.g. sound
  propagation, LOS), a `PropKind::Door` + collider gate goes here.
- **Multi-z-level buildings.** Roof cut-away + ghost pass is the
  first z-level UX; basements + upper floors ship as their own
  mapgen entries and need multi-layer stamp support.

## Frontier — cdda-specific open questions

1. **Can the world hold the whole corpus?** `place_nested`,
   vehicles, monsters, multi-tile overmap specials — which
   unhandled features must land to grow past lone buildings?
2. **When does a building stop being scenery?** Which CDDA flags
   become behaviour — `TRANSPARENT`→glass, doors open/close,
   `CONTAINER`/`SEALED`→loot? The hand-placed jukebox game-side is
   a CDDA furniture entry we could *read* instead of *author*.
3. **Should the generator BE CDDA's grammar?** `palette.rs` already
   rolls per-building seeds through variant palettes. How far up
   (block, town) does authored parameter/distribution reach as the
   world generator vs our per-chunk hash?
4. **Can you go down and up?** CDDA ships basements + upper floors
   as their own mapgen. What's descend/ascend in an iso voxel
   world?
5. **Do lone buildings become towns?** CDDA authors roads, blocks,
   connected specials. Settlement coherence — more authored data
   through the same pipeline, or new placement logic?

## Corpus — dependency, not vendored

CDDA JSON is **fetched at build time** from a pinned release,
never checked into git.

- Version pin: [`RELEASE`](./RELEASE) (currently `0.I`).
- Commit pin: [`COMMIT`](./COMMIT) — the exact sha the release
  points at, verified after fetch.
- Manifest of files to embed: [`files.txt`](./files.txt).
- License: CC-BY-SA 3.0. See [`ATTRIBUTION.md`](./ATTRIBUTION.md).

Two fetch paths:

- **Nix** (CI + `nix develop`): `nix build .#cdda-src` from
  `game/flake.nix` realises the pinned subtrees; `$CDDA_SRC` points
  `build.rs` at the store path.
- **Bare cargo**: `tools/fetch.sh` sparse-clones the same subtrees
  into gitignored `.cdda-src/`. `build.rs` falls back to this when
  `$CDDA_SRC` is unset. Verifies the fetched HEAD matches
  [`COMMIT`](./COMMIT) (a moved tag or compromised mirror would
  otherwise swap the corpus silently).

**Adding a building** is one line in `files.txt` + registration in
`src/building.rs` (`assemble_building(...)` + a `specs` entry in
`load_building_templates`). The flake's content hash does **not**
change on corpus edits — the fetch is decoupled from the manifest.

Bumping `RELEASE`: edit the file, blank the flake `hash =`, run
`nix build .#cdda-src` — Nix prints the real hash to paste in. All
reviewable in the diff.

## Structure

```
crates/cdda/
├── Cargo.toml
├── RELEASE                 # CDDA release name (e.g. 0.I)
├── COMMIT                  # exact sha the release points at
├── files.txt               # manifest of embedded mapgen + palette files
├── ATTRIBUTION.md          # CC-BY-SA 3.0
├── build.rs                # copies referenced files from $CDDA_SRC to OUT_DIR/cdda/
├── assets/buildings/       # hand-authored mapgens (currently just shed.json)
├── tools/fetch.sh          # bare-cargo fallback fetch
└── src/
    ├── lib.rs              # public API + module map
    ├── template.rs         # Prop / PropKind / Template — the wire shape
    ├── parse.rs            # CDDA JSON walkers
    ├── cells.rs            # char → PropKind mapping
    ├── palette.rs          # nested palette resolver with per-building seed
    ├── placement.rs        # mapgen → Template (walls, windows, doors, fences)
    ├── building.rs         # canonical building assembly + registry
    ├── chunks.rs           # which chunks host a building (pure hash)
    ├── hash.rs             # wang_hash — small pure primitive
    └── bin/coverage.rs     # `cargo run --bin cdda-coverage` — corpus report
```
