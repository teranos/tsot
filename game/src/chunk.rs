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
use crate::trees::{self, TreeTrunk};

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

/// A yard cleared of trees just beyond a building's own footprint.
const TREE_YARD_MARGIN: f32 = 90.0;
/// A tree field (orchard) clears a much wider ring so its planted rows
/// stand alone in the open, not crowded by the wild forest around them.
const ORCHARD_YARD_MARGIN: f32 = 380.0;

/// How many chunks a building's props can reach from its anchor chunk.
/// A prop at `half` from the anchor (which sits at a chunk centre) lands
/// at most `(half + CHUNK_SIZE/2)/CHUNK_SIZE` chunks away, rounded up.
/// Single-tile buildings give 1; a multi-tile school reaches further.
pub fn building_reach(buildings: &crate::buildings::BuildingTemplates) -> i32 {
    let max_half = buildings.half_extents.iter().copied().fold(0.0_f32, f32::max);
    ((max_half + CHUNK_SIZE * 0.5) / CHUNK_SIZE).ceil() as i32
}

/// Every building whose footprint can reach chunk `c`: anchors within
/// `reach` chunks that carry a building, with the deterministic template
/// index + rotation. Pure — every peer computes the same set, so a
/// multi-tile building assembles identically no matter which chunk you
/// approach it from.
pub fn buildings_reaching(
    c: ChunkCoord,
    buildings: &crate::buildings::BuildingTemplates,
    reach: i32,
) -> Vec<(Vec3, usize, u8)> {
    let mut out = Vec::new();
    if buildings.is_empty() {
        return out;
    }
    for dx in -reach..=reach {
        for dz in -reach..=reach {
            let a = ChunkCoord { x: c.x + dx, z: c.z + dz };
            if let Some(anchor) = cdda::building_anchor_in_chunk(a.x, a.z, CHUNK_SIZE) {
                let idx = cdda::building_index(a.x, a.z, buildings.len());
                out.push((anchor, idx, cdda::building_rotation(a.x, a.z)));
            }
        }
    }
    out
}

/// Deterministic trees in a chunk — the ground-plane base positions of
/// every tree whose cell this chunk owns, minus any that would grow
/// through a nearby building's footprint (a multi-tile building's yard
/// reaches into neighbouring chunks, so we clear against every building
/// that reaches this one, sized to its own footprint). Pure.
pub fn trees_in_chunk(c: ChunkCoord, buildings: &crate::buildings::BuildingTemplates) -> Vec<(Vec3, f32)> {
    let reach = building_reach(buildings);
    let yards: Vec<(Vec3, f32)> = buildings_reaching(c, buildings, reach)
        .into_iter()
        .map(|(anchor, idx, _rot)| {
            let t = &buildings.templates[idx];
            // A tree field (orchard: trees, no props) clears a wide open
            // ring; a building just clears its immediate yard.
            let margin = if t.props.is_empty() && !t.trees.is_empty() {
                ORCHARD_YARD_MARGIN
            } else {
                TREE_YARD_MARGIN
            };
            (anchor, buildings.half_extents[idx] + margin)
        })
        .collect();
    let mut out = Vec::new();
    for lx in 0..CHUNK_CELLS {
        for lz in 0..CHUNK_CELLS {
            let ix = c.x * CHUNK_CELLS + lx;
            let iz = c.z * CHUNK_CELLS + lz;
            if let Some((base, height)) = trees::tree_at_cell(ix, iz) {
                let in_a_yard = yards.iter().any(|(a, half)| {
                    (base.x - a.x).abs() < *half && (base.z - a.z).abs() < *half
                });
                if !in_a_yard {
                    out.push((base, height));
                }
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

/// Spawn the trunk + foliage entities for one tree base position, with
/// an explicit species — procedural trees pass `species_for_pos`,
/// authored (CDDA) trees pass the species their map named.
fn spawn_tree(
    commands: &mut Commands,
    base: Vec3,
    height: f32,
    species: &'static crate::tree_mesh::TreeSpecies,
    stump: bool,
) -> Entity {
    // A stump's collider is knee-high, not a full trunk column.
    let col_y = if stump { 30.0 } else { 200.0 };
    commands
        .spawn((
            TreeTrunk { height, species, stump },
            Position(Vec3::new(base.x, 0.0, base.z)),
            AabbCollider::cuboid(Vec3::new(24.0, col_y, 24.0)),
        ))
        .id()
}

/// Streaming system — keep exactly the chunks around the player loaded.
/// Cheap on frames where the player stays in range (just set diffs);
/// spawns/despawns a chunk's worth of trees only when a boundary is
/// crossed.
pub fn stream_chunks(
    mut commands: Commands,
    player_q: Query<&Position, With<PlayerMarker>>,
    mut loaded: ResMut<LoadedChunks>,
    buildings: Res<crate::buildings::BuildingTemplates>,
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
        for (base, height) in trees_in_chunk(c, &buildings) {
            let species = crate::tree_mesh::species_for_pos(base.x, base.z);
            let stump = trees::is_stump_at(base.x, base.z);
            entities.push(spawn_tree(&mut commands, base, height, species, stump));
        }
        if let Some(anchor) = campsite::campsite_in_chunk(c) {
            entities.extend(stamp_template(
                &mut commands,
                &campsite::campsite_template(),
                anchor,
            ));
        }
        // Buildings: stamp the props of every building reaching this chunk
        // that land INSIDE it. A single-tile building stamps entirely into
        // its own chunk; a multi-tile one has each chunk stamp its own
        // slice, so it loads/unloads per-chunk and never despawns from
        // under the player (mirrors CDDA generating one om tile at a time).
        let reach = building_reach(&buildings);
        let (cx0, cz0) = (c.x as f32 * CHUNK_SIZE, c.z as f32 * CHUNK_SIZE);
        for (anchor, idx, rot) in buildings_reaching(c, &buildings, reach) {
            // Cheap AABB reject: skip buildings whose footprint can't
            // overlap this chunk (so a single-tile building only ever
            // rotates + stamps for its own chunk).
            let half = buildings.half_extents[idx];
            if anchor.x + half < cx0
                || anchor.x - half > cx0 + CHUNK_SIZE
                || anchor.z + half < cz0
                || anchor.z - half > cz0 + CHUNK_SIZE
            {
                continue;
            }
            let rotated = crate::template::rotate_template(&buildings.templates[idx], rot);
            entities.extend(crate::template::stamp_template_where(
                &mut commands,
                &rotated,
                anchor,
                |w| world_to_chunk(w) == c,
            ));
            // Authored trees (e.g. an apple orchard) — spawn each tree that
            // lands in this chunk, with the species its map named. Same
            // per-chunk keep filter as the props, so a multi-tile tree
            // field streams slice by slice.
            for tp in &rotated.trees {
                let base = anchor + tp.offset;
                if world_to_chunk(base) != c {
                    continue;
                }
                let species = crate::tree_mesh::species_for_kind(tp.kind);
                let stump = matches!(tp.kind, crate::template::TreeKind::Stump);
                let height = trees::authored_height(base.x, base.z, species);
                entities.push(spawn_tree(&mut commands, base, height, species, stump));
            }
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

    fn no_buildings() -> crate::buildings::BuildingTemplates {
        crate::buildings::BuildingTemplates::default()
    }

    #[test]
    fn trees_in_chunk_are_deterministic() {
        let c = ChunkCoord { x: 3, z: 4 };
        let b = no_buildings();
        assert_eq!(trees_in_chunk(c, &b), trees_in_chunk(c, &b));
    }

    #[test]
    fn adjacent_chunks_do_not_share_trees() {
        let b = no_buildings();
        let a = trees_in_chunk(ChunkCoord { x: 3, z: 4 }, &b);
        let bb = trees_in_chunk(ChunkCoord { x: 4, z: 4 }, &b);
        for pa in &a {
            assert!(!bb.iter().any(|pb| pb == pa), "a tree is in two chunks");
        }
    }

    #[test]
    fn trees_dont_grow_through_a_building() {
        let (buildings, _) = crate::buildings::BuildingTemplates::load();
        // Find a chunk that carries a building.
        let mut found = None;
        'search: for x in 0..80 {
            for z in 0..80 {
                let c = ChunkCoord { x, z };
                if let Some(a) = cdda::building_anchor_in_chunk(c.x, c.z, CHUNK_SIZE) {
                    found = Some((c, a));
                    break 'search;
                }
            }
        }
        let (c, anchor) = found.expect("a building chunk should exist");
        let idx = cdda::building_index(c.x, c.z, buildings.len());
        let half = buildings.half_extents[idx];
        for (base, _) in trees_in_chunk(c, &buildings) {
            let inside = (base.x - anchor.x).abs() < half && (base.z - anchor.z).abs() < half;
            assert!(!inside, "tree at {base:?} grows through building at {anchor:?}");
        }
    }

    #[test]
    fn a_forest_chunk_carries_trees() {
        // A chunk well away from the clearing should hold some trees.
        assert!(!trees_in_chunk(ChunkCoord { x: 6, z: 6 }, &no_buildings()).is_empty());
    }

    #[test]
    fn a_building_wider_than_a_chunk_spans_and_reaches_multiple_chunks() {
        use crate::template::{Prop, PropKind, Template};
        // A wall line ~3 chunks wide.
        let span = (CHUNK_SIZE * 1.2) as i32;
        let props: Vec<Prop> = (-span..=span)
            .step_by(80)
            .map(|x| Prop::at(Vec3::new(x as f32, 0.0, 0.0), PropKind::Wall))
            .collect();
        let big = Template { props, trees: vec![], ..Default::default() };
        let half = big.props.iter().fold(0.0_f32, |m, p| m.max(p.offset.x.abs()));
        let bt = crate::buildings::BuildingTemplates { templates: vec![big.clone()], half_extents: vec![half] };
        // Reach must extend past a single chunk.
        assert!(building_reach(&bt) >= 1, "reach should cover a multi-chunk building");
        // Anchored at a chunk centre, its props land in more than one chunk.
        let anchor = Vec3::new(0.5 * CHUNK_SIZE, 0.0, 0.5 * CHUNK_SIZE);
        let chunks: std::collections::BTreeSet<_> =
            big.props.iter().map(|p| world_to_chunk(anchor + p.offset)).collect();
        assert!(chunks.len() > 1, "a building wider than a chunk must span chunks: {}", chunks.len());
    }

    #[test]
    fn multi_tile_building_survives_when_its_anchor_chunk_unloads() {
        use bevy_ecs::system::RunSystemOnce;
        use crate::template::{Prop, PropKind, StructureProp, Template};
        // A wall line ~1.5 chunks wide in +/- x — bigger than one chunk.
        let span = (CHUNK_SIZE * 1.5) as i32;
        let props: Vec<Prop> = (-span..=span)
            .step_by(80)
            .map(|x| Prop::at(Vec3::new(x as f32, 0.0, 0.0), PropKind::Wall))
            .collect();
        let bt = crate::buildings::BuildingTemplates {
            templates: vec![Template { props, trees: vec![], ..Default::default() }],
            half_extents: vec![span as f32],
        };
        // Anchor it at a real (deterministic) building chunk.
        let mut bc = None;
        'outer: for x in 0..200 {
            for z in 0..200 {
                let c = ChunkCoord { x, z };
                if cdda::building_anchor_in_chunk(c.x, c.z, CHUNK_SIZE).is_some() {
                    bc = Some(c);
                    break 'outer;
                }
            }
        }
        let bc = bc.expect("a building chunk exists");
        let anchor = cdda::building_anchor_in_chunk(bc.x, bc.z, CHUNK_SIZE).unwrap();

        let mut world = World::new();
        world.insert_resource(LoadedChunks::default());
        world.insert_resource(bt);
        let player = world.spawn((PlayerMarker, Position(anchor))).id();
        let count_walls = |w: &mut World| {
            let mut q = w.query::<&StructureProp>();
            q.iter(w).count()
        };

        world.run_system_once(stream_chunks).unwrap();
        assert!(count_walls(&mut world) > 0, "building should stream in near the anchor");

        // Walk to the building's far wing: the anchor chunk drops out of
        // range, but the wing chunk (now loaded) owns its slice — so the
        // building is still there rather than despawning from under you.
        world.get_mut::<Position>(player).unwrap().0 = anchor + Vec3::new(span as f32, 0.0, 0.0);
        world.run_system_once(stream_chunks).unwrap();
        assert!(
            !world.resource::<LoadedChunks>().0.contains_key(&bc),
            "the anchor chunk should have unloaded"
        );
        assert!(
            count_walls(&mut world) > 0,
            "the building slice at the wing survives the anchor chunk unloading"
        );
    }

    #[test]
    fn streaming_loads_around_player_and_stays_bounded_on_move() {
        use bevy_ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.insert_resource(LoadedChunks::default());
        world.insert_resource(crate::buildings::BuildingTemplates::load().0);
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
