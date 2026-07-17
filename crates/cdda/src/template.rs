//! Structure templates — the pure data types + pure resolve/rotate/
//! digest operations. No ECS. No RNG. No render.
//!
//! `Template` is a bag of `Prop`s, each with an offset from an anchor.
//! `resolve_placements(template, anchor)` is the deterministic core:
//! given a template + anchor it returns each prop's world position and
//! kind. Two peers stamping the same template at the same anchor
//! resolve to byte-identical output — the P2P invariant.
//!
//! The ECS wrapper that spawns Bevy entities from a Template lives in
//! the consumer crate (see `game/src/template.rs`), not here.

use bevy_math::Vec3;

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
    /// A toilet. White ceramic box, blocks movement.
    Toilet,
}

impl PropKind {
    /// Glass window variants — drawn translucent, in their own pass.
    pub fn is_window(self) -> bool {
        matches!(self, PropKind::Window | PropKind::WindowNS | PropKind::WindowEW)
    }
}

/// A tree species tag — the thin, framework-free vocabulary CDDA's
/// `t_tree_*` terrain maps onto. The consumer crate (`game`) turns each
/// into its richer `TreeSpecies` geometry/palette; this crate stays
/// render-free, so it only names the kind. Fieldless + `Copy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeKind {
    Apple,
    Pine,
    Oak,
    Birch,
    Willow,
    /// Any tree we don't map to a specific species yet.
    Generic,
}

/// Fixed u8 tag per `TreeKind` for `stable_digest` — pinned like
/// `prop_kind_tag`, so reordering the enum never shifts a digest.
pub(crate) fn tree_kind_tag(k: TreeKind) -> u8 {
    match k {
        TreeKind::Apple => 0,
        TreeKind::Pine => 1,
        TreeKind::Oak => 2,
        TreeKind::Birch => 3,
        TreeKind::Willow => 4,
        TreeKind::Generic => 5,
    }
}

/// One authored tree, positioned relative to the template's anchor —
/// the parallel to `Prop` for the tree layer. Trees are a distinct
/// entity/render path from props, so they ride their own vector rather
/// than a `PropKind` variant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreePlacement {
    pub offset: Vec3,
    pub kind: TreeKind,
}

/// One prop, positioned relative to the template's anchor.
#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub offset: Vec3,
    pub kind: PropKind,
    /// Colour override; `None` → the kind's default appearance.
    pub color: Option<[f32; 3]>,
    /// Size override; `None` → the kind's default size (from the render
    /// side's per-kind appearance table). Used by the run-based wall
    /// importer to emit one long WallEW/WallNS for a continuous straight
    /// run of cells instead of one prop per cell — with a single prop
    /// there's no seam between the pieces because they ARE the same piece.
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
    /// Authored trees this template places (from `t_tree_*` terrain).
    /// Empty for buildings; populated for a tree field like an orchard.
    pub trees: Vec<TreePlacement>,
}

impl Template {
    /// Cross-platform, cross-compile stable digest of the resolved
    /// template. FNV-1a over an explicit byte serialization — floats as
    /// `to_le_bytes`, the prop kind as a fixed u8 tag (not the enum's
    /// automatic discriminant), colour as tag + straight bytes. The
    /// determinism property test (resolve twice, require equal) uses
    /// this to prove the resolver is a pure function of its input.
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
        // Tree layer — same explicit byte mixing so the determinism
        // property covers authored trees too.
        for t in &self.trees {
            mix_bytes(&t.offset.x.to_le_bytes(), &mut mix);
            mix_bytes(&t.offset.y.to_le_bytes(), &mut mix);
            mix_bytes(&t.offset.z.to_le_bytes(), &mut mix);
            mix(tree_kind_tag(t.kind));
        }
        h
    }
}

/// Fixed u8 tag per `PropKind` — pinned here so a refactor that
/// reorders the enum doesn't silently change every stable digest.
pub(crate) fn prop_kind_tag(k: PropKind) -> u8 {
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
        PropKind::Toilet => 14,
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
            let size = p.size.map(|s| {
                if q % 2 == 1 { Vec3::new(s.z, s.y, s.x) } else { s }
            });
            Prop { offset: Vec3::new(rx, p.offset.y, rz), kind, color: p.color, size }
        })
        .collect();
    // Trees rotate with the template — their positions turn, but a tree's
    // appearance is rotation-invariant so the kind is unchanged.
    let trees = t
        .trees
        .iter()
        .map(|tp| {
            let (x, z) = (tp.offset.x, tp.offset.z);
            let (rx, rz) = match q {
                1 => (-z, x),
                2 => (-x, -z),
                3 => (z, -x),
                _ => (x, z),
            };
            TreePlacement {
                offset: Vec3::new(rx, tp.offset.y, rz),
                kind: tp.kind,
            }
        })
        .collect();
    Template { props, trees }
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
            trees: vec![],
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
    fn rotate_turns_offsets_and_swaps_wall_orientation() {
        let t = Template {
            props: vec![Prop::at(Vec3::new(10.0, 5.0, 0.0), PropKind::WallNS)],
            trees: vec![],
        };
        let r1 = rotate_template(&t, 1);
        assert_eq!(r1.props[0].offset, Vec3::new(0.0, 5.0, 10.0));
        assert_eq!(r1.props[0].kind, PropKind::WallEW);
        let r4 = rotate_template(&t, 4);
        assert_eq!(r4.props[0].offset, t.props[0].offset);
        assert_eq!(r4.props[0].kind, t.props[0].kind);
    }

    #[test]
    fn resolution_is_deterministic() {
        let t = Template {
            props: vec![Prop::at(Vec3::new(7.0, 8.0, 9.0), PropKind::Campfire)],
            trees: vec![],
        };
        let a = resolve_placements(&t, Vec3::splat(3.0));
        let b = resolve_placements(&t, Vec3::splat(3.0));
        assert_eq!(a, b);
    }
}
