# BEVY

- Bevy 0.19.
- Started from NiklasEi/bevy_game_template: https://github.com/NiklasEi/bevy_game_template
- CI needed.

## Common Bevy setup

- [ ] bevy-inspector-egui
- [ ] bevy_asset_loader
- [ ] bevy_kira_audio
- [ ] avian
- [ ] sickle_ui
- [ ] bevy_atmosphere
- [ ] bevy_water
- [ ] bevy_mod_outline
- [ ] bevy diagnostic events → roam trace bus
- [ ] bevy/dynamic_linking native build
- [ ] CI

## v0.5 — Bevy + 3D voxel render

The world model is already 3D-shaped (`tile_at(x, y, z)`, `surface_z`, the voxel framing in `roam/README.md` "What I want"); the render catches up. Bevy owns the frame loop + wgpu pipeline + camera + mesh + lighting; `bevy_ecs` lights up the object-identity axiom (entities + Location components — pickup is `remove::<OnGround>().insert::<InInventory>()` on the same entity). egui drops out entirely: the existing `roam/src/ui/mod.rs` rewrites straight into Bevy's in-core `bevy_ui`. Worldgen + net + identity unchanged. See `roam/docs/adr/0003-bevy.md`.

- [x] 0.5.0 — Bevy is set up. `universe/`.
- [ ] 0.5.1 — each UI element built fresh in `universe/` as a standalone `bevy_ui` widget showing dummy data, in parallel to roam's existing eframe UI (which keeps running). Order: clock → build watermark → hotbar (Node + 9 dummy slots) → spawn menu (right-click popup) → font picker (popup + preview). One at a time, each landed independently. Raw `bevy_ui` only; `sickle_ui` not pre-adopted. No shared styling abstractions yet. Wiring widgets to real roam state + dropping eframe is a separate slice — 0.5.1.1 or folded into 0.5.3, TBD.
- [ ] 0.5.2 — rewrite `render_gl` as Bevy meshes + materials. World stays top-down 2D for this slice; visual baseline preserved before the dimensionality shift.
- [ ] 0.5.3 — `World` migrates to ECS. Player, peers, flowers, cards become entities + components. The object-identity axiom in `roam/docs/UI.md` falls out of one query + one system.
- [ ] 0.5.4 — click-to-pickup + drag + pickup animation. Falls out of ECS + Bevy input + position interpolation + `bevy_picking`.
- [ ] 0.5.5 — 3D camera + voxel rendering. `bevy_voxel_world` if 0.18-ready, otherwise `block-mesh-rs` + hand-rolled chunking.

## Decisions

- [ ] camera / projection
- [ ] voxel approach
- [ ] flake consolidation
- [ ] 0.18 → 0.19 timing
- [ ] bevy_egui yes / no

## References

- https://bevy.org/learn/migration-guides/0-18-to-0-19/
- https://bevy.org/learn/quick-start/getting-started/setup/#compile-with-performance-optimizations
- https://github.com/NiklasEi/bevy_game_template
- https://github.com/splashdust/bevy_voxel_world
- https://github.com/bonsairobo/block-mesh-rs
- https://github.com/UmbraLuminosa/sickle_ui
- https://github.com/Jondolf/avian
- https://github.com/NiklasEi/bevy_kira_audio
- https://github.com/NiklasEi/bevy_asset_loader
