//! Which chunks host a building, which template variant they pick, and
//! how they're rotated — all pure hashes so every peer sees the same
//! world.

use bevy_math::Vec3;

use crate::chunk::{CHUNK_SIZE, ChunkCoord};
use crate::hash::wang_hash;

/// Buildings are rarer than campsites — roughly 1 chunk in 20.
const BUILDING_CHUNK_CHANCE: u32 = u32::MAX / 20;
const BUILDING_SALT: u32 = 0xB1D6_5175;
const BUILDING_PICK_SALT: u32 = 0xB1D6_9CE5;
const BUILDING_ROT_SALT: u32 = 0xB1D6_2074;
/// Square half-extent around a building kept clear of trees, so the
/// forest doesn't grow through the walls — reads as a yard around it.
pub const BUILDING_FOOTPRINT_HALF: f32 = 1050.0;
/// Keep buildings well clear of the central clearing + trail; they're
/// big, so exclude their footprint reach plus a margin.
const BUILDING_CLEARING_EXCLUSION: f32 = 2000.0;
const BUILDING_TRAIL_HALF: f32 = 220.0 + BUILDING_FOOTPRINT_HALF;

/// Does this chunk carry a building, and where? Pure. Anchor is the
/// chunk centre — buildings aren't jittered, so they fit inside their
/// own chunk. `None` inside the central clearing or the trail corridor.
pub fn building_anchor_in_chunk(c: ChunkCoord) -> Option<Vec3> {
    if wang_hash(c.x, c.z, BUILDING_SALT) >= BUILDING_CHUNK_CHANCE {
        return None;
    }
    let anchor = Vec3::new(
        (c.x as f32 + 0.5) * CHUNK_SIZE,
        0.0,
        (c.z as f32 + 0.5) * CHUNK_SIZE,
    );
    if anchor.x.hypot(anchor.z) < BUILDING_CLEARING_EXCLUSION {
        return None;
    }
    if anchor.z > 500.0 && anchor.x.abs() < BUILDING_TRAIL_HALF {
        return None;
    }
    Some(anchor)
}

/// Which cached template a building-chunk uses — a deterministic hash
/// pick, so the same chunk is the same building on every peer.
pub fn building_index(c: ChunkCoord, num: usize) -> usize {
    (wang_hash(c.x, c.z, BUILDING_PICK_SALT) as usize) % num
}

/// Deterministic quarter-turn rotation (0..4) for a building-chunk, so
/// two buildings of the same type face different ways.
pub fn building_rotation(c: ChunkCoord) -> u8 {
    (wang_hash(c.x, c.z, BUILDING_ROT_SALT) % 4) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buildings_are_rare_deterministic_and_center_clear() {
        let c = ChunkCoord { x: 7, z: -3 };
        assert_eq!(building_anchor_in_chunk(c), building_anchor_in_chunk(c));
        let (mut n, mut total) = (0, 0);
        for x in -25..25 {
            for z in -25..25 {
                total += 1;
                if let Some(a) = building_anchor_in_chunk(ChunkCoord { x, z }) {
                    n += 1;
                    assert!(a.x.hypot(a.z) >= BUILDING_CLEARING_EXCLUSION);
                }
            }
        }
        assert!(n > 0, "some chunks should carry a building");
        assert!(n < total / 8, "buildings should be rare: {n}/{total}");
    }
}
