//! Structure templates — a group of props placed relative to an
//! anchor, stamped into the ECS world in one call.
//!
//! This is the primitive underneath every placed structure. The
//! campfire is its degenerate one-prop case (see `campfire.rs`); a
//! campsite, a CDDA-imported building, and a player-placed structure
//! all become `Template`s that feed the same `stamp_template`.
//!
//! `resolve_placements` is the deterministic, ECS-free core: template
//! + anchor → world position + kind of every prop. Keeping it pure
//! (like `room::touch_drag_to_plane`) means a placement is unit-
//! testable, and — crucially for the P2P world — two peers stamping
//! the same template at the same anchor resolve to the identical set
//! of world positions with no shared RNG.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::campfire::{self, Campfire};
use crate::physics::{AabbCollider, Position};

/// What a single template entry becomes when stamped. Grows as the
/// world gains props (tents, chairs, walls, floors, CDDA furniture…).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropKind {
    Campfire,
    Chair,
    Table,
    /// Wall segment filling a whole CDDA tile — corner/junction/isolated.
    Wall,
    /// Wall running N–S (along Z): long in Z, thin in X.
    WallNS,
    /// Wall running E–W (along X): long in X, thin in Z.
    WallEW,
    /// Flat roof slab; elevation carried in the prop's y offset.
    Roof,
}

/// Render/identity tag for a static structure prop (chair, table, and
/// CDDA furniture later). `scene.rs` maps the kind to a colour + size.
/// The campfire is deliberately NOT a `StructureProp` — it renders
/// through its own flickering path.
#[derive(Component, Clone, Copy, Debug)]
pub struct StructureProp {
    pub kind: PropKind,
}

/// One prop, positioned relative to the template's anchor.
#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub offset: Vec3,
    pub kind: PropKind,
}

/// A named group of props. The anchor is supplied at stamp time, so
/// the same template can be placed anywhere — by a hash-seeded
/// procedural pass, or by a player — and every peer stamps it
/// identically.
#[derive(Clone, Debug, Default)]
pub struct Template {
    pub props: Vec<Prop>,
}

/// Pure placement resolution: `template` + `anchor` → the world
/// position and kind of every prop. No ECS, no side effects, no RNG.
pub fn resolve_placements(template: &Template, anchor: Vec3) -> Vec<(Vec3, PropKind)> {
    template
        .props
        .iter()
        .map(|prop| (anchor + prop.offset, prop.kind))
        .collect()
}

/// Stamp every prop of `template` into the world at `anchor`. Thin ECS
/// wrapper over `resolve_placements`; each `PropKind` spawns its
/// bundle. The per-kind bundles mirror what each module spawned when
/// it placed itself directly, so the render/collision contract is
/// unchanged.
pub fn stamp_template(commands: &mut Commands, template: &Template, anchor: Vec3) {
    for (pos, kind) in resolve_placements(template, anchor) {
        match kind {
            PropKind::Campfire => {
                commands.spawn((
                    Campfire {
                        intensity: campfire::BASE_INTENSITY,
                    },
                    Position(pos),
                    AabbCollider {
                        half_extents: campfire::COLLIDER_HALF,
                    },
                ));
            }
            PropKind::Chair => {
                // Decor — no collider; you can step around a camp chair.
                commands.spawn((StructureProp { kind: PropKind::Chair }, Position(pos)));
            }
            PropKind::Table => {
                commands.spawn((
                    StructureProp { kind: PropKind::Table },
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(64.0, 28.0, 64.0)),
                ));
            }
            PropKind::Wall => {
                // Solid, one CDDA tile square. Sizes across all wall
                // kinds match scene.rs's appearances.
                commands.spawn((
                    StructureProp { kind: PropKind::Wall },
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(80.0, 220.0, 80.0)),
                ));
            }
            PropKind::WallNS => {
                commands.spawn((
                    StructureProp { kind: PropKind::WallNS },
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(24.0, 220.0, 80.0)),
                ));
            }
            PropKind::WallEW => {
                commands.spawn((
                    StructureProp { kind: PropKind::WallEW },
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(80.0, 220.0, 24.0)),
                ));
            }
            PropKind::Roof => {
                // Overhead — no collider; the player walks under it.
                commands.spawn((StructureProp { kind: PropKind::Roof }, Position(pos)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_translates_each_prop_by_the_anchor() {
        let t = Template {
            props: vec![
                Prop { offset: Vec3::new(1.0, 2.0, 3.0), kind: PropKind::Campfire },
                Prop { offset: Vec3::new(-5.0, 0.0, 10.0), kind: PropKind::Campfire },
            ],
        };
        let anchor = Vec3::new(100.0, 0.0, -100.0);
        let out = resolve_placements(&t, anchor);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], (Vec3::new(101.0, 2.0, -97.0), PropKind::Campfire));
        assert_eq!(out[1], (Vec3::new(95.0, 0.0, -90.0), PropKind::Campfire));
    }

    #[test]
    fn empty_template_resolves_to_nothing() {
        assert!(resolve_placements(&Template::default(), Vec3::ZERO).is_empty());
    }

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

    #[test]
    fn resolution_is_deterministic() {
        // Same template + anchor → identical placements every call.
        // Guards the P2P invariant: no hidden RNG can creep in.
        let t = Template {
            props: vec![Prop { offset: Vec3::new(7.0, 8.0, 9.0), kind: PropKind::Campfire }],
        };
        let a = resolve_placements(&t, Vec3::splat(3.0));
        let b = resolve_placements(&t, Vec3::splat(3.0));
        assert_eq!(a, b);
    }
}
