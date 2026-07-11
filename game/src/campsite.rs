//! Hash-seeded procedural campsites — a fire ringed by camp chairs
//! with a couple of tables, stamped at deterministic clearings across
//! the world. Same world for every peer, no shared RNG: the anchors
//! come from the shared `hash::wang_hash`, exactly like the forest, so
//! two peers compute the identical set of campsites independently.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::hash::{jitter, wang_hash};
use crate::room;
use crate::template::{stamp_template, Prop, PropKind, Template};

/// Coarse placement grid — much larger than the tree cell (80) so
/// campsites are sparse and spread out.
const CELL: f32 = 700.0;
/// ~1-in-6 cells carries a campsite, before the exclusions below.
const SPAWN_THRESHOLD: u32 = u32::MAX / 6;
/// Central clearing (the rave floor) half-extent — kept campsite-free.
const CLEARING_HALF: f32 = 500.0;
const CLEARING_EXCLUSION: f32 = CLEARING_HALF + 300.0;
/// Keep campsites off the south-going trail so the walk-in stays open.
const TRAIL_CORRIDOR_HALF: f32 = 220.0;
/// Salt distinct from the forest's so campsites and trees don't
/// correlate cell-for-cell.
const CAMPSITE_SALT: u32 = 0x0CA3_5175;

/// The campsite layout: a fire at the centre, four camp chairs ringing
/// it, two tables off to the side. Offsets are world units relative to
/// the anchor. Swap this for a CDDA-imported layout later — the
/// placement + stamp machinery doesn't change.
pub fn campsite_template() -> Template {
    const CHAIR_R: f32 = 90.0;
    let mut props = vec![Prop { offset: Vec3::ZERO, kind: PropKind::Campfire }];
    for (dx, dz) in [(CHAIR_R, 0.0), (-CHAIR_R, 0.0), (0.0, CHAIR_R), (0.0, -CHAIR_R)] {
        props.push(Prop { offset: Vec3::new(dx, 0.0, dz), kind: PropKind::Chair });
    }
    props.push(Prop { offset: Vec3::new(160.0, 0.0, 70.0), kind: PropKind::Table });
    props.push(Prop { offset: Vec3::new(-150.0, 0.0, -60.0), kind: PropKind::Table });
    Template { props }
}

/// Deterministic campsite anchors. Pure — same result on every peer,
/// every call, no RNG. A cell carries a campsite when its hash gates
/// in and it clears the central clearing + the trail corridor.
pub fn campsite_anchors() -> Vec<Vec3> {
    let floor_half = room::FLOOR_HALF;
    let cells_per_side = (floor_half * 2.0 / CELL) as i32;
    let origin = -floor_half;
    let mut anchors = Vec::new();
    for ix in 0..cells_per_side {
        for iz in 0..cells_per_side {
            let cell_x = origin + ix as f32 * CELL + CELL / 2.0;
            let cell_z = origin + iz as f32 * CELL + CELL / 2.0;
            // Skip the central clearing.
            if cell_x.hypot(cell_z) < CLEARING_EXCLUSION {
                continue;
            }
            // Skip the south-going trail corridor.
            if cell_z > CLEARING_HALF && cell_x.abs() < TRAIL_CORRIDOR_HALF {
                continue;
            }
            // Hash gate — only some cells carry a campsite.
            if wang_hash(ix, iz, CAMPSITE_SALT) >= SPAWN_THRESHOLD {
                continue;
            }
            // Sub-cell jitter so campsites don't read as a grid.
            let jx = jitter(ix, iz, 1) * (CELL * 0.3);
            let jz = jitter(ix, iz, 2) * (CELL * 0.3);
            anchors.push(Vec3::new(cell_x + jx, 0.0, cell_z + jz));
        }
    }
    anchors
}

/// Startup system — stamp a campsite at every anchor.
pub fn setup_campsites(mut commands: Commands) {
    let template = campsite_template();
    for anchor in campsite_anchors() {
        stamp_template(&mut commands, &template, anchor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_are_nonempty_and_deterministic() {
        let a = campsite_anchors();
        assert!(!a.is_empty(), "expected at least one campsite");
        assert!(a.len() < 60, "campsite count unreasonable: {}", a.len());
        assert_eq!(a, campsite_anchors(), "placement must be deterministic");
    }

    #[test]
    fn anchors_avoid_the_central_clearing() {
        for p in campsite_anchors() {
            assert!(
                p.x.hypot(p.z) >= CLEARING_HALF,
                "campsite at {p:?} intrudes on the clearing",
            );
        }
    }

    #[test]
    fn campsite_template_has_a_fire_ringed_by_chairs_and_tables() {
        let t = campsite_template();
        let count = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
        assert_eq!(count(PropKind::Campfire), 1);
        assert_eq!(count(PropKind::Chair), 4);
        assert_eq!(count(PropKind::Table), 2);
        // The fire is the anchor — zero offset.
        let fire = t.props.iter().find(|p| p.kind == PropKind::Campfire).unwrap();
        assert_eq!(fire.offset, Vec3::ZERO);
    }
}
