# BEVY

Pinned at `=0.19.0` in `rave/Cargo.toml`. CI builds the wasm bundle on
every rave-branch push and deploys to https://rave.sbvh.nl/.

## Pending

- [ ] Bump Bevy when 0.20 ships. Watch the migration guide; the
  asset-loader + Mesh3d/MeshMaterial3d shape often changes between
  minor versions.
- [ ] Tree-shake the feature list. `2d_api`, `2d_bevy_render`, `scene`
  are enabled but no rave code uses them; cutting them shrinks the
  wasm bundle.
- [ ] `wasm-opt -O4 --strip-debug` on the release `rave_bg.wasm` to
  halve the first-load payload.

## Decisions made

- **Camera / projection** — 3D top-down at ~50° pitch, follow camera
  trails the player from `(0, 300, 250)` offset. See `rave/src/room.rs`.
- **Voxel approach** — not voxel. Floor is a single `Plane3d`; props
  (DJ booth, speakers, bar, walls, truss, strobes) are individual
  meshes spawned by `floorplan.rs`.
- **HDR + Bloom** — camera carries `Hdr` + `Bloom::default()` from
  `bevy::post_process::bloom`. Strobes + truss spots read as nightclub
  lights, not matte. Base ambient is intentionally dim so the colour
  cycling actually shows.
- **`bevy_egui`** — not used. The in-canvas drawer renders via Bevy's
  own UI plugin (`ui_api` + `ui_bevy_render` features). Text-only,
  no widgets.
- **Asset loading** — TBD when the first non-procedural asset lands
  (Poly Pizza humanoid models for player avatar replacement).
  `bevy_asset_loader` may earn its keep then; not before.

## References

- https://docs.rs/bevy/0.19.0/bevy/ — API docs (authoritative; the
  pbt control in `controls/local/tsot-roam.pbt` insists on sourcing
  from here, not training memory)
- https://github.com/bevyengine/bevy/tree/v0.19.0/examples — examples
- https://bevy.org/learn/migration-guides/0-18-to-0-19/ — migration
  notes from the version below ours (kept for context until the 0.20
  guide ships)
