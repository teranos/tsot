//! Chunked streaming world — the mechanism that makes the world span
//! far and wide at *constant* cost. Space is an infinite grid of
//! chunks; only the chunks in a small radius around the player are
//! "loaded" (their trees spawned as entities). As the player moves,
//! new chunks load and far ones unload, so the live entity count is
//! O(radius²), independent of world size.
//!
//! The forest itself is never stored — it's `trees::tree_at_cell`, a
//! pure hash. A chunk owns an integer block of cells, so generating a
//! chunk is just running the hash over its cells; "streaming" a region
//! is computation, not disk I/O. Determinism means every peer loads
//! the identical chunk.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::campsite;
use crate::physics::{AabbCollider, Position, PlayerMarker};
use crate::template::stamp_template;
use crate::trees::{self, TreeFoliage, TreeTrunk};

/// Cells per chunk side. A chunk is an integer block of forest cells,
/// so cells never straddle a chunk boundary — each tree belongs to
/// exactly one chunk.
pub const CHUNK_CELLS: i32 = 20;
/// Chunk side in world units (== CHUNK_CELLS whole cells).
pub const CHUNK_SIZE: f32 = CHUNK_CELLS as f32 * trees::CELL;
/// Load the player's chunk plus this many rings around it. 1 → a 3×3
/// block, comfortably larger than the camera view so chunks load
/// before they'd be visible.
pub const LOAD_RADIUS: i32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ChunkCoord {
    pub x: i32,
    pub z: i32,
}

/// Which chunk a world position falls in.
pub fn world_to_chunk(pos: Vec3) -> ChunkCoord {
    ChunkCoord {
        x: (pos.x / CHUNK_SIZE).floor() as i32,
        z: (pos.z / CHUNK_SIZE).floor() as i32,
    }
}

/// The square block of chunks within `radius` of `center` — the set
/// that should be loaded.
pub fn active_chunks(center: ChunkCoord, radius: i32) -> Vec<ChunkCoord> {
    let mut out = Vec::new();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            out.push(ChunkCoord { x: center.x + dx, z: center.z + dz });
        }
    }
    out
}

/// Deterministic trees in a chunk — the ground-plane base positions of
/// every tree whose cell this chunk owns. Pure.
pub fn trees_in_chunk(c: ChunkCoord) -> Vec<Vec3> {
    let mut out = Vec::new();
    for lx in 0..CHUNK_CELLS {
        for lz in 0..CHUNK_CELLS {
            let ix = c.x * CHUNK_CELLS + lx;
            let iz = c.z * CHUNK_CELLS + lz;
            if let Some(base) = trees::tree_at_cell(ix, iz) {
                out.push(base);
            }
        }
    }
    out
}

/// Live chunk → spawned entities. `BTreeMap` (not `HashMap`) so there's
/// no default-hasher entropy dependency in the wasm build, and load
/// order is deterministic.
#[derive(Resource, Default)]
pub struct LoadedChunks(pub BTreeMap<ChunkCoord, Vec<Entity>>);

/// Spawn the trunk + foliage entities for one tree base position.
fn spawn_tree(commands: &mut Commands, base: Vec3) -> [Entity; 2] {
    let trunk = commands
        .spawn((
            TreeTrunk,
            Position(Vec3::new(base.x, trees::TRUNK_Y, base.z)),
            AabbCollider::cuboid(Vec3::new(12.0, 60.0, 12.0)),
        ))
        .id();
    let foliage = commands
        .spawn((
            TreeFoliage,
            Position(Vec3::new(base.x, trees::FOLIAGE_Y, base.z)),
        ))
        .id();
    [trunk, foliage]
}

/// Streaming system — keep exactly the chunks around the player loaded.
/// Cheap on frames where the player stays in range (just set diffs);
/// spawns/despawns a chunk's worth of trees only when a boundary is
/// crossed.
pub fn stream_chunks(
    mut commands: Commands,
    player_q: Query<&Position, With<PlayerMarker>>,
    mut loaded: ResMut<LoadedChunks>,
) {
    let Some(player) = player_q.iter().next() else {
        return;
    };
    let center = world_to_chunk(player.0);
    let want = active_chunks(center, LOAD_RADIUS);

    // Unload chunks that fell out of range.
    let stale: Vec<ChunkCoord> = loaded
        .0
        .keys()
        .filter(|c| !want.contains(c))
        .copied()
        .collect();
    for c in stale {
        if let Some(entities) = loaded.0.remove(&c) {
            for e in entities {
                commands.entity(e).despawn();
            }
        }
    }

    // Load newly in-range chunks.
    for c in want {
        if loaded.0.contains_key(&c) {
            continue;
        }
        let mut entities = Vec::new();
        for base in trees_in_chunk(c) {
            entities.extend(spawn_tree(&mut commands, base));
        }
        if let Some(anchor) = campsite::campsite_in_chunk(c) {
            entities.extend(stamp_template(
                &mut commands,
                &campsite::campsite_template(),
                anchor,
            ));
        }
        loaded.0.insert(c, entities);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_to_chunk_partitions_space() {
        assert_eq!(world_to_chunk(Vec3::ZERO), ChunkCoord { x: 0, z: 0 });
        assert_eq!(
            world_to_chunk(Vec3::new(CHUNK_SIZE * 1.5, 0.0, -CHUNK_SIZE * 0.5)),
            ChunkCoord { x: 1, z: -1 }
        );
    }

    #[test]
    fn active_chunks_is_a_square_block() {
        let a = active_chunks(ChunkCoord { x: 5, z: -2 }, 1);
        assert_eq!(a.len(), 9);
        assert!(a.contains(&ChunkCoord { x: 5, z: -2 }));
        assert!(a.contains(&ChunkCoord { x: 6, z: -1 }));
        assert!(!a.contains(&ChunkCoord { x: 7, z: -2 }));
    }

    #[test]
    fn trees_in_chunk_are_deterministic() {
        let c = ChunkCoord { x: 3, z: 4 };
        assert_eq!(trees_in_chunk(c), trees_in_chunk(c));
    }

    #[test]
    fn adjacent_chunks_do_not_share_trees() {
        let a = trees_in_chunk(ChunkCoord { x: 3, z: 4 });
        let b = trees_in_chunk(ChunkCoord { x: 4, z: 4 });
        for pa in &a {
            assert!(!b.iter().any(|pb| pb == pa), "a tree is in two chunks");
        }
    }

    #[test]
    fn a_forest_chunk_carries_trees() {
        // A chunk well away from the clearing should hold some trees.
        assert!(!trees_in_chunk(ChunkCoord { x: 6, z: 6 }).is_empty());
    }

    #[test]
    fn streaming_loads_around_player_and_stays_bounded_on_move() {
        use bevy_ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.insert_resource(LoadedChunks::default());
        let player = world
            .spawn((PlayerMarker, Position(Vec3::new(0.0, 20.0, 0.0))))
            .id();

        world.run_system_once(stream_chunks).unwrap();
        assert_eq!(world.resource::<LoadedChunks>().0.len(), 9);
        let count_trunks = |w: &mut World| {
            let mut q = w.query::<&TreeTrunk>();
            q.iter(w).count()
        };
        let first = count_trunks(&mut world);
        assert!(first > 0, "trees should stream in around the player");

        // Re-running in place is idempotent — no new chunks, no growth.
        world.run_system_once(stream_chunks).unwrap();
        assert_eq!(world.resource::<LoadedChunks>().0.len(), 9);
        assert_eq!(count_trunks(&mut world), first);

        // Move far and re-run: the loaded set shifts, count stays bounded.
        world.get_mut::<Position>(player).unwrap().0 =
            Vec3::new(CHUNK_SIZE * 20.0, 20.0, CHUNK_SIZE * 20.0);
        world.run_system_once(stream_chunks).unwrap();
        assert_eq!(world.resource::<LoadedChunks>().0.len(), 9, "still 3x3");
        assert!(
            world
                .resource::<LoadedChunks>()
                .0
                .contains_key(&ChunkCoord { x: 20, z: 20 }),
            "new region loaded"
        );
    }
}
