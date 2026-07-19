//! ECS wrapper over the cdda-crate `Template` primitive.
//!
//! `cdda::Template` is the pure data — a bag of `Prop`s to place at an
//! anchor, no ECS, no side effects. `stamp_template` walks that bag and
//! spawns Bevy entities per `PropKind`: colliders, positions, and the
//! `StructureProp` tag the render pass reads to draw the right shape.
//! Two peers stamping the same template at the same anchor get
//! byte-identical world state (see `cdda::resolve_placements` — the
//! pure resolve that stamp is a Bevy-flavoured wrapper of).

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

// Re-export the pure types so game code can `use crate::template::PropKind`
// without knowing whether they live here or in the cdda seam crate.
pub use cdda::template::{
    Prop, PropKind, Template, TreeKind, TreePlacement, resolve_placements, rotate_template,
};

use crate::campfire::{self, Campfire};
use crate::physics::{AabbCollider, Position};

/// Render/identity tag for a static structure prop (chair, table, and
/// CDDA furniture later). `scene.rs` maps the kind to a colour + size.
/// The campfire is deliberately NOT a `StructureProp` — it renders
/// through its own flickering path.
#[derive(Component, Clone, Copy, Debug)]
pub struct StructureProp {
    pub kind: PropKind,
    /// Colour override; `None` → the kind's default appearance.
    pub color: Option<[f32; 3]>,
    /// Size override; `None` → the kind's default size (from `scene.rs`).
    pub size: Option<Vec3>,
}

/// Stamp every prop of `template` into the world at `anchor`. Thin ECS
/// wrapper over `cdda::resolve_placements`; each `PropKind` spawns its
/// bundle. Returns the spawned entities, so a streaming caller can
/// despawn a placed structure when its chunk unloads.
pub fn stamp_template(commands: &mut Commands, template: &Template, anchor: Vec3) -> Vec<Entity> {
    stamp_template_where(commands, template, anchor, |_| true)
}

/// Like `stamp_template`, but only spawns props whose *world* position
/// (`anchor + offset`) satisfies `keep`. This is what lets a multi-tile
/// building be distributed across the chunks it spans: each chunk stamps
/// only the props that land inside it, so the building loads/unloads
/// per-chunk (mirroring how CDDA generates one overmap tile at a time)
/// instead of despawning wholesale when a single anchor chunk unloads.
pub fn stamp_template_where(
    commands: &mut Commands,
    template: &Template,
    anchor: Vec3,
    keep: impl Fn(Vec3) -> bool,
) -> Vec<Entity> {
    let mut spawned = Vec::new();
    for prop in &template.props {
        let pos = anchor + prop.offset;
        if !keep(pos) {
            continue;
        }
        let sp = |kind: PropKind| StructureProp { kind, color: prop.color, size: prop.size };
        let entity = match prop.kind {
            PropKind::Campfire => commands
                .spawn((
                    Campfire {
                        intensity: campfire::BASE_INTENSITY,
                    },
                    Position(pos),
                    AabbCollider {
                        half_extents: campfire::COLLIDER_HALF,
                    },
                ))
                .id(),
            // Decor — no collider; you can step around a camp chair.
            PropKind::Chair => commands.spawn((sp(PropKind::Chair), Position(pos))).id(),
            PropKind::Table => commands
                .spawn((
                    sp(PropKind::Table),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(64.0, 28.0, 64.0)),
                ))
                .id(),
            // Solid, one CDDA tile square. Wall sizes match scene.rs.
            // The run-based importer stores the run's length in prop.size
            // (one prop per continuous straight run), so the collider
            // needs to match the actual prop size when present.
            PropKind::Wall => commands
                .spawn((
                    sp(PropKind::Wall),
                    Position(pos),
                    AabbCollider::cuboid(prop.size.unwrap_or(Vec3::new(80.0, 220.0, 80.0))),
                ))
                .id(),
            PropKind::WallNS => commands
                .spawn((
                    sp(PropKind::WallNS),
                    Position(pos),
                    AabbCollider::cuboid(prop.size.unwrap_or(Vec3::new(24.0, 220.0, 80.0))),
                ))
                .id(),
            PropKind::WallEW => commands
                .spawn((
                    sp(PropKind::WallEW),
                    Position(pos),
                    AabbCollider::cuboid(prop.size.unwrap_or(Vec3::new(80.0, 220.0, 24.0))),
                ))
                .id(),
            // Overhead — no collider; the player walks under it.
            PropKind::Roof => commands.spawn((sp(PropKind::Roof), Position(pos))).id(),
            PropKind::Furniture => commands
                .spawn((
                    sp(PropKind::Furniture),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(50.0, 70.0, 50.0)),
                ))
                .id(),
            // Glass — solid to bump into, like the wall it sits in.
            PropKind::Window => commands
                .spawn((
                    sp(PropKind::Window),
                    Position(pos),
                    AabbCollider::cuboid(prop.size.unwrap_or(Vec3::new(80.0, 220.0, 80.0))),
                ))
                .id(),
            PropKind::WindowNS => commands
                .spawn((
                    sp(PropKind::WindowNS),
                    Position(pos),
                    AabbCollider::cuboid(prop.size.unwrap_or(Vec3::new(24.0, 220.0, 80.0))),
                ))
                .id(),
            PropKind::WindowEW => commands
                .spawn((
                    sp(PropKind::WindowEW),
                    Position(pos),
                    AabbCollider::cuboid(prop.size.unwrap_or(Vec3::new(80.0, 220.0, 24.0))),
                ))
                .id(),
            // Fence — short (60 tall) barrier, blocks movement.
            // Collider spans the whole cell along its axis so the fence
            // line is unbroken even with the see-through visual.
            PropKind::Fence => commands
                .spawn((
                    sp(PropKind::Fence),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(80.0, 60.0, 80.0)),
                ))
                .id(),
            PropKind::FenceNS => commands
                .spawn((
                    sp(PropKind::FenceNS),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(8.0, 60.0, 80.0)),
                ))
                .id(),
            PropKind::FenceEW => commands
                .spawn((
                    sp(PropKind::FenceEW),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(80.0, 60.0, 8.0)),
                ))
                .id(),
            // Toilet — small white ceramic block, blocks movement.
            PropKind::Toilet => commands
                .spawn((
                    sp(PropKind::Toilet),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(36.0, 50.0, 44.0)),
                ))
                .id(),
        };
        spawned.push(entity);
    }
    spawned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campfire_template_stamps_one_campfire_at_the_anchor() {
        // Drive the real Startup system against a bare World and confirm
        // the campfire-as-template refactor still yields exactly one
        // Campfire entity, at SPAWN_POS — the render/collision contract
        // scene.rs depends on.
        use crate::campfire;
        use bevy_ecs::prelude::World;
        use bevy_ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.run_system_once(campfire::setup_campfire).unwrap();
        let mut q = world.query::<(&Position, &campfire::Campfire)>();
        let hits: Vec<Vec3> = q.iter(&world).map(|(p, _)| p.0).collect();
        assert_eq!(hits.len(), 1, "expected exactly one campfire entity");
        assert_eq!(hits[0], campfire::SPAWN_POS);
    }
}
