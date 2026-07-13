//! Structure templates — a group of props placed relative to an
//! anchor, stamped into the ECS world in one call.
//!
//! This is the primitive underneath every placed structure. The
//! campfire is its degenerate one-prop case (see `campfire.rs`); a
//! campsite, a CDDA-imported building, and a player-placed structure
//! all become `Template`s that feed the same `stamp_template`.
//!
//! `resolve_placements` is the deterministic, ECS-free core: given a
//! template and an anchor it returns the world position + kind of every
//! prop. Keeping it pure (like `room::touch_drag_to_plane`) means a
//! placement is unit-testable, and — crucially for the P2P world — two
//! peers stamping the same template at the same anchor resolve to the
//! identical set of world positions with no shared RNG.

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
    /// Generic solid furniture (bed, dresser, fridge, …) — a box.
    Furniture,
    /// Translucent glass window filling a tile (corner/junction/isolated).
    /// Rendered in a separate alpha-blended pass, see-through from
    /// outside; solid to walk into like the wall it sits in.
    Window,
    /// Glass window running N–S (along Z): long in Z, thin in X.
    WindowNS,
    /// Glass window running E–W (along X): long in X, thin in Z.
    WindowEW,
}

impl PropKind {
    /// Glass window variants — drawn translucent, in their own pass.
    pub fn is_window(self) -> bool {
        matches!(self, PropKind::Window | PropKind::WindowNS | PropKind::WindowEW)
    }
}

/// Render/identity tag for a static structure prop (chair, table, and
/// CDDA furniture later). `scene.rs` maps the kind to a colour + size.
/// The campfire is deliberately NOT a `StructureProp` — it renders
/// through its own flickering path.
#[derive(Component, Clone, Copy, Debug)]
pub struct StructureProp {
    pub kind: PropKind,
    /// Colour override; `None` → the kind's default appearance.
    pub color: Option<[f32; 3]>,
}

/// One prop, positioned relative to the template's anchor.
#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub offset: Vec3,
    pub kind: PropKind,
    /// Colour override; `None` → the kind's default appearance.
    pub color: Option<[f32; 3]>,
}

impl Prop {
    /// A prop with the default appearance for its kind.
    pub fn at(offset: Vec3, kind: PropKind) -> Self {
        Self { offset, kind, color: None }
    }
    /// A prop with an explicit colour (e.g. a wall tinted by material).
    pub fn colored(offset: Vec3, kind: PropKind, color: [f32; 3]) -> Self {
        Self { offset, kind, color: Some(color) }
    }
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

/// Rotate a template by `quarter_turns` × 90° in the XZ plane, so
/// hash-placed buildings face different ways. Oriented walls swap
/// NS↔EW on odd turns so a wall run stays thin across its length.
pub fn rotate_template(t: &Template, quarter_turns: u8) -> Template {
    let q = quarter_turns % 4;
    let props = t
        .props
        .iter()
        .map(|p| {
            let (x, z) = (p.offset.x, p.offset.z);
            let (rx, rz) = match q {
                1 => (-z, x),
                2 => (-x, -z),
                3 => (z, -x),
                _ => (x, z),
            };
            let kind = if q % 2 == 1 {
                match p.kind {
                    PropKind::WallNS => PropKind::WallEW,
                    PropKind::WallEW => PropKind::WallNS,
                    PropKind::WindowNS => PropKind::WindowEW,
                    PropKind::WindowEW => PropKind::WindowNS,
                    k => k,
                }
            } else {
                p.kind
            };
            Prop { offset: Vec3::new(rx, p.offset.y, rz), kind, color: p.color }
        })
        .collect();
    Template { props }
}

/// Stamp every prop of `template` into the world at `anchor`. Thin ECS
/// wrapper over `resolve_placements`; each `PropKind` spawns its
/// bundle. The per-kind bundles mirror what each module spawned when
/// it placed itself directly, so the render/collision contract is
/// unchanged.
/// Returns the spawned entities, so a streaming caller can despawn a
/// placed structure when its chunk unloads. Callers that place
/// permanent structures can ignore the return.
pub fn stamp_template(commands: &mut Commands, template: &Template, anchor: Vec3) -> Vec<Entity> {
    let mut spawned = Vec::new();
    for prop in &template.props {
        let pos = anchor + prop.offset;
        // StructureProp carrying this prop's colour override (walls
        // tinted by material; None elsewhere → kind default).
        let sp = |kind: PropKind| StructureProp { kind, color: prop.color };
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
            PropKind::Wall => commands
                .spawn((
                    sp(PropKind::Wall),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(80.0, 220.0, 80.0)),
                ))
                .id(),
            PropKind::WallNS => commands
                .spawn((
                    sp(PropKind::WallNS),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(24.0, 220.0, 80.0)),
                ))
                .id(),
            PropKind::WallEW => commands
                .spawn((
                    sp(PropKind::WallEW),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(80.0, 220.0, 24.0)),
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
                    AabbCollider::cuboid(Vec3::new(80.0, 220.0, 80.0)),
                ))
                .id(),
            PropKind::WindowNS => commands
                .spawn((
                    sp(PropKind::WindowNS),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(24.0, 220.0, 80.0)),
                ))
                .id(),
            PropKind::WindowEW => commands
                .spawn((
                    sp(PropKind::WindowEW),
                    Position(pos),
                    AabbCollider::cuboid(Vec3::new(80.0, 220.0, 24.0)),
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
    fn resolve_translates_each_prop_by_the_anchor() {
        let t = Template {
            props: vec![
                Prop::at(Vec3::new(1.0, 2.0, 3.0), PropKind::Campfire),
                Prop::at(Vec3::new(-5.0, 0.0, 10.0), PropKind::Campfire),
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
    fn rotate_turns_offsets_and_swaps_wall_orientation() {
        let t = Template {
            props: vec![Prop::at(Vec3::new(10.0, 5.0, 0.0), PropKind::WallNS)],
        };
        // 90°: (x,z)=(10,0) → (0,10); NS wall → EW; y unchanged.
        let r1 = rotate_template(&t, 1);
        assert_eq!(r1.props[0].offset, Vec3::new(0.0, 5.0, 10.0));
        assert_eq!(r1.props[0].kind, PropKind::WallEW);
        // Four quarter-turns is the identity.
        let r4 = rotate_template(&t, 4);
        assert_eq!(r4.props[0].offset, t.props[0].offset);
        assert_eq!(r4.props[0].kind, t.props[0].kind);
    }

    #[test]
    fn resolution_is_deterministic() {
        // Same template + anchor → identical placements every call.
        // Guards the P2P invariant: no hidden RNG can creep in.
        let t = Template {
            props: vec![Prop::at(Vec3::new(7.0, 8.0, 9.0), PropKind::Campfire)],
        };
        let a = resolve_placements(&t, Vec3::splat(3.0));
        let b = resolve_placements(&t, Vec3::splat(3.0));
        assert_eq!(a, b);
    }
}
