# Handover — `renderer/tree-lifestages`

## Why this branch exists

Death is a state of the tree, not a kind of tree. A dying oak is
still an oak. `TreeTrunk.stump: bool` was the first shape of this
mistake — the second was `DEAD` living as its own species.
`LifeStage` collapses both onto the trunk as an enum orthogonal
to species.

See `docs/TREES.md § Life stages` for the model.

## What ships

- `LifeStage { Sapling, Mature, Snag, Stump, Fallen }` on
  `TreeTrunk`, default `Mature`.
- Snag draws the species wood mesh (greyer tint), no leaves, no
  fruit, tips marked `is_dead`.
- Stump replaces the removed `stump: bool`, same behaviour.
- CDDA bridge: `TreeKind::Stump → Stump`, `TreeKind::Dead → Snag`.
- Sapling + Fallen: enum variants reserved, not drawn.

Threaded through `SceneSnapshot.trees`, `chunk::spawn_tree`,
`snapshot_to_mesh_instances`, and the species-wood emit.

Locked in by `tree_emit::a_snag_is_a_leafless_dead_tree_of_its_species`.

## Deferred

- **Retire the `DEAD` species from `tree_mesh.rs`.** The bridge
  still calls `species_for_kind(Dead)`. Point `TreeKind::Dead` at
  a real living species (oak by default) so the Snag renders as a
  dead-of-its-species tree, then delete `DEAD`.
- **Procedural death.** Roll a per-tile "died naturally" bit in
  `trees.rs` next to `is_stump_at` so procedural forests grow
  their own snags — today only authored CDDA tiles become Snags.
- **Sapling + Fallen draw paths.** Enum variants exist so the
  data model is honest; the render side is empty.

## Commits

- `038377f` docs first — TREES.md declares LifeStage.
- `f79a0f9` failing test — Snag is leafless, still species wood.
- `a93899d` implementation — enum, threading, wood tint, canopy skip.

## State

Pushed. PR not opened yet — waiting on CI green for `a93899d`.
