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
    /// Fence — a short, see-through barrier. Blocks movement (a real
    /// fence you'd have to go around), but does NOT seal a building's
    /// interior for flood-fill: an area enclosed by fences stays
    /// exterior (a yard, not a room). Rendered as two stacked rails
    /// so the gap between is visible.
    Fence,
    /// Fence running N–S (along Z).
    FenceNS,
    /// Fence running E–W (along X).
    FenceEW,
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
    /// Size override; `None` → the kind's default size (from `scene.rs`).
    pub size: Option<Vec3>,
}

/// One prop, positioned relative to the template's anchor.
#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub offset: Vec3,
    pub kind: PropKind,
    /// Colour override; `None` → the kind's default appearance.
    pub color: Option<[f32; 3]>,
    /// Size override; `None` → the kind's default size (from `scene.rs`).
    /// Used by the run-based wall importer to emit one long WallEW/WallNS
    /// for a continuous straight run of cells instead of one prop per cell
    /// — with a single prop there's no seam between the pieces because
    /// they ARE the same piece.
    pub size: Option<Vec3>,
}

impl Prop {
    /// A prop with the default appearance for its kind.
    pub fn at(offset: Vec3, kind: PropKind) -> Self {
        Self { offset, kind, color: None, size: None }
    }
    /// A prop with an explicit colour (e.g. a wall tinted by material).
    pub fn colored(offset: Vec3, kind: PropKind, color: [f32; 3]) -> Self {
        Self { offset, kind, color: Some(color), size: None }
    }
    /// A prop with an explicit size (a run of cells emitted as one prop).
    pub fn sized(offset: Vec3, kind: PropKind, color: Option<[f32; 3]>, size: Vec3) -> Self {
        Self { offset, kind, color, size: Some(size) }
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

impl Template {
    /// Cross-platform, cross-compile stable digest of the resolved
    /// template. FNV-1a over an explicit byte serialization — floats as
    /// `to_le_bytes`, the prop kind as a fixed u8 tag (not the enum's
    /// automatic discriminant), colour as tag + straight bytes. Used by
    /// the golden-master test to catch determinism drift: two peers
    /// resolving the same building must produce identical props, and any
    /// refactor that shifts even one byte should trip the test.
    pub fn stable_digest(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |b: u8| {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        };
        let mix_bytes = |bytes: &[u8], mix: &mut dyn FnMut(u8)| {
            for &b in bytes {
                mix(b);
            }
        };
        for p in &self.props {
            mix_bytes(&p.offset.x.to_le_bytes(), &mut mix);
            mix_bytes(&p.offset.y.to_le_bytes(), &mut mix);
            mix_bytes(&p.offset.z.to_le_bytes(), &mut mix);
            mix(prop_kind_tag(p.kind));
            match p.color {
                None => mix(0),
                Some(c) => {
                    mix(1);
                    mix_bytes(&c[0].to_le_bytes(), &mut mix);
                    mix_bytes(&c[1].to_le_bytes(), &mut mix);
                    mix_bytes(&c[2].to_le_bytes(), &mut mix);
                }
            }
            match p.size {
                None => mix(0),
                Some(s) => {
                    mix(1);
                    mix_bytes(&s.x.to_le_bytes(), &mut mix);
                    mix_bytes(&s.y.to_le_bytes(), &mut mix);
                    mix_bytes(&s.z.to_le_bytes(), &mut mix);
                }
            }
        }
        h
    }
}

/// Fixed u8 tag per `PropKind` — pinned here so a refactor that
/// reorders the enum doesn't silently change every golden digest.
fn prop_kind_tag(k: PropKind) -> u8 {
    match k {
        PropKind::Campfire => 0,
        PropKind::Chair => 1,
        PropKind::Table => 2,
        PropKind::Wall => 3,
        PropKind::WallNS => 4,
        PropKind::WallEW => 5,
        PropKind::Roof => 6,
        PropKind::Furniture => 7,
        PropKind::Window => 8,
        PropKind::WindowNS => 9,
        PropKind::WindowEW => 10,
        PropKind::Fence => 11,
        PropKind::FenceNS => 12,
        PropKind::FenceEW => 13,
    }
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
                    PropKind::FenceNS => PropKind::FenceEW,
                    PropKind::FenceEW => PropKind::FenceNS,
                    k => k,
                }
            } else {
                p.kind
            };
            // Size follows kind: an odd-turn rotation swaps a run's long
            // and thin axes, so size.x and size.z swap too.
            let size = p.size.map(|s| {
                if q % 2 == 1 { Vec3::new(s.z, s.y, s.x) } else { s }
            });
            Prop { offset: Vec3::new(rx, p.offset.y, rz), kind, color: p.color, size }
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
        // StructureProp carrying this prop's colour override (walls
        // tinted by material; None elsewhere → kind default).
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
