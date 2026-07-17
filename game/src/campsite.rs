//! Hash-seeded procedural campsites — a fire ringed by camp chairs
//! with a couple of tables. Placed per chunk (like the forest), so
//! they scatter endlessly across the streamed, boundless world rather
//! than only within a central region. Same world for every peer: the
//! placement is pure `hash::wang_hash` of the chunk coordinate, no RNG.
//! The chunk streamer (`chunk::stream_chunks`) spawns a chunk's
//! campsite when it loads and despawns it when it unloads.

use bevy_math::Vec3;

use crate::chunk::{ChunkCoord, CHUNK_SIZE};
use crate::hash::{jitter, wang_hash};
use crate::template::{Prop, PropKind, Template};

/// Roughly 1 chunk in 10 carries a campsite — sparse enough that you
/// rarely see more than one at a time, common enough to stumble on.
const CAMPSITE_CHUNK_CHANCE: u32 = u32::MAX / 10;
/// Central clearing (the rave floor) half-extent — kept campsite-free.
const CLEARING_HALF: f32 = 500.0;
const CLEARING_EXCLUSION: f32 = CLEARING_HALF + 300.0;
/// Keep campsites off the south-going trail so the walk-in stays open.
const TRAIL_CORRIDOR_HALF: f32 = 220.0;
/// Salt distinct from the forest's so campsites and trees don't
/// correlate chunk-for-chunk.
const CAMPSITE_SALT: u32 = 0x0CA3_5175;

/// The campsite layout: a fire at the centre, four camp chairs ringing
/// it, two tables off to the side. Offsets are world units relative to
/// the anchor. Swap for a CDDA-imported layout later — the placement +
/// stamp machinery doesn't change.
pub fn campsite_template() -> Template {
    const CHAIR_R: f32 = 90.0;
    let mut props = vec![Prop::at(Vec3::ZERO, PropKind::Campfire)];
    for (dx, dz) in [(CHAIR_R, 0.0), (-CHAIR_R, 0.0), (0.0, CHAIR_R), (0.0, -CHAIR_R)] {
        props.push(Prop::at(Vec3::new(dx, 0.0, dz), PropKind::Chair));
    }
    props.push(Prop::at(Vec3::new(160.0, 0.0, 70.0), PropKind::Table));
    props.push(Prop::at(Vec3::new(-150.0, 0.0, -60.0), PropKind::Table));
    Template { props, trees: vec![] }
}

/// The campsite anchor for a chunk, if the hash gates one in and it
/// clears the central clearing + trail. Pure + deterministic — every
/// peer generates the identical campsite for a chunk.
pub fn campsite_in_chunk(c: ChunkCoord) -> Option<Vec3> {
    // Only some chunks carry one.
    if wang_hash(c.x, c.z, CAMPSITE_SALT) >= CAMPSITE_CHUNK_CHANCE {
        return None;
    }
    // A jittered anchor inside the chunk (kept off the edges so its
    // props don't spill into a neighbour).
    let cx = (c.x as f32 + 0.5) * CHUNK_SIZE;
    let cz = (c.z as f32 + 0.5) * CHUNK_SIZE;
    let anchor = Vec3::new(
        cx + jitter(c.x, c.z, 7) * (CHUNK_SIZE * 0.3),
        0.0,
        cz + jitter(c.x, c.z, 8) * (CHUNK_SIZE * 0.3),
    );
    // Keep clear of the central clearing + trail corridor.
    if anchor.x.hypot(anchor.z) < CLEARING_EXCLUSION {
        return None;
    }
    if anchor.z > CLEARING_HALF && anchor.x.abs() < TRAIL_CORRIDOR_HALF {
        return None;
    }
    Some(anchor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campsite_in_chunk_is_deterministic() {
        let c = ChunkCoord { x: 4, z: -9 };
        assert_eq!(campsite_in_chunk(c), campsite_in_chunk(c));
    }

    #[test]
    fn campsites_are_sparse_but_present() {
        let (mut count, mut total) = (0, 0);
        for x in -20..20 {
            for z in -20..20 {
                total += 1;
                if campsite_in_chunk(ChunkCoord { x, z }).is_some() {
                    count += 1;
                }
            }
        }
        assert!(count > 0, "some chunks should carry a campsite");
        assert!(count < total / 3, "campsites should be sparse: {count}/{total}");
    }

    #[test]
    fn campsites_avoid_the_central_clearing() {
        for x in -3..3 {
            for z in -3..3 {
                if let Some(a) = campsite_in_chunk(ChunkCoord { x, z }) {
                    assert!(
                        a.x.hypot(a.z) >= CLEARING_EXCLUSION,
                        "campsite {a:?} intrudes on the clearing"
                    );
                }
            }
        }
    }

    #[test]
    fn campsite_template_has_a_fire_ringed_by_chairs_and_tables() {
        let t = campsite_template();
        let count = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
        assert_eq!(count(PropKind::Campfire), 1);
        assert_eq!(count(PropKind::Chair), 4);
        assert_eq!(count(PropKind::Table), 2);
        let fire = t.props.iter().find(|p| p.kind == PropKind::Campfire).unwrap();
        assert_eq!(fire.offset, Vec3::ZERO);
    }
}
