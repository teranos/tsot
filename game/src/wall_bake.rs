//! Walls-on-mesh slice 4: bake a building's `WallGraph` into coloured,
//! draw-ready mesh parts (see `game/docs/RENDER.md`).
//!
//! Colour model (from design discussion): exterior wall faces carry
//! the material colour the graph authored (brick / wood / concrete via
//! CDDA palettes); INTERIOR faces are coloured per room — rooms are
//! derived from the graph's enclosure (flood fill over the cell grid),
//! and each building seeds its own coherent palette, so houses vary
//! from one another while the rooms inside one house fit together.
//!
//! Cut-away model: the iso camera direction is fixed, so which faces
//! are camera-facing is a static property of the bake. Triangles are
//! ordered far-first; `near_start` marks where camera-facing geometry
//! begins. Outside a building the whole range draws; inside, drawing
//! `0..near_start` cuts the near walls without touching a buffer.
//! A code comment records the for-now assumption: a rotating camera
//! breaks this bake.

use bevy_math::Vec3;
use cdda::{WallGraph, WallNode};

use crate::tree_mesh::MeshVertex;

/// One colour group of a building's wall mesh. `indices[..near_start]`
/// are the far (never-cut) triangles; `indices[near_start..]` face the
/// fixed iso camera (outward normal with x+z > 0) and are skipped when
/// the player is inside the building.
pub struct WallPart {
    pub color: [f32; 3],
    pub verts: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub near_start: usize,
    /// Camera-depth (x+z, template-local) of each NEAR triangle, in
    /// the order they sit after `near_start` — ascending. Lets the
    /// draw cut exactly at the player's depth: draw near triangles
    /// whose depth ≤ the player's, skip the rest.
    pub near_depths: Vec<f32>,
    /// The SAME near triangles, depth-DESCENDING: the ghost pass draws
    /// the first (near_total − visible_near) of these — always a
    /// prefix, because the web mesh crossing can only draw prefixes —
    /// and that prefix is exactly the set the opaque draw skipped.
    pub ghost_indices: Vec<u32>,
}

impl WallPart {
    /// The per-frame draw ranges: (opaque index count, ghost index
    /// count). Outside a building everything is opaque and nothing
    /// ghosts; inside, near triangles up to `local_depth` stay opaque
    /// and the rest — exactly the skipped set — ghost. `local_depth`
    /// is the player's camera depth relative to the building anchor
    /// (see `local_depth`).
    pub fn draw_counts(&self, inside: bool, local_depth: f32) -> (u32, u32) {
        if !inside {
            return (self.indices.len() as u32, 0);
        }
        let visible_near = self.near_depths.partition_point(|d| *d <= local_depth);
        let near_total = self.near_depths.len();
        (
            (self.near_start + visible_near * 3) as u32,
            ((near_total - visible_near) * 3) as u32,
        )
    }
}

/// The player's camera depth relative to a building anchor, with the
/// cube path's historical +40 margin — the cut threshold for
/// `WallPart::draw_counts`.
pub fn local_depth(anchor: Vec3, player: Vec3) -> f32 {
    (player.x - anchor.x) + (player.z - anchor.z) + 40.0
}

/// A building's baked wall mesh: colour-grouped, cut-away-ordered.
pub struct WallBake {
    pub parts: Vec<WallPart>,
}

/// Interior room palette — muted, cohering domestic colours. A
/// building picks a rotation into this palette from its seed, and
/// rooms take consecutive entries: rooms differ within a house, and
/// the whole house stays in one family.
const ROOM_PALETTE: [[f32; 3]; 8] = [
    [0.86, 0.74, 0.52], // warm plaster
    [0.56, 0.72, 0.52], // sage
    [0.80, 0.60, 0.46], // clay
    [0.50, 0.64, 0.80], // dusty blue
    [0.82, 0.78, 0.62], // bone
    [0.74, 0.58, 0.66], // mauve
    [0.66, 0.74, 0.50], // olive
    [0.84, 0.68, 0.42], // ochre
];

/// Derive the rooms of a wall graph: cells enclosed by the wall line,
/// as connected components. Returns (cell → room id) as a map keyed by
/// the graph's cell coordinates, and the room count. Deterministic:
/// components are numbered in row-major scan order.
pub fn rooms_of(g: &WallGraph) -> (std::collections::HashMap<(i32, i32), usize>, usize) {
    use std::collections::HashMap;
    let mut out = HashMap::new();
    if g.nodes.is_empty() {
        return (out, 0);
    }
    // Offsets sit at half-cell multiples when the grid dimension is
    // odd ((c − W/2)·CELL), so `round` would collapse neighbours at
    // ±0.5 boundaries; `floor` maps every parity onto consecutive
    // integers.
    let cell = |n: &WallNode| -> (i32, i32) {
        (
            (n.offset.x / cdda::CDDA_TILE).floor() as i32,
            (n.offset.z / cdda::CDDA_TILE).floor() as i32,
        )
    };
    let walls: std::collections::HashSet<(i32, i32)> = g.nodes.iter().map(cell).collect();
    let (x0, x1) = g.nodes.iter().map(cell).fold((i32::MAX, i32::MIN), |(a, b), c| (a.min(c.0), b.max(c.0)));
    let (z0, z1) = g.nodes.iter().map(cell).fold((i32::MAX, i32::MIN), |(a, b), c| (a.min(c.1), b.max(c.1)));
    // Flood the exterior from a ring outside the bbox.
    let (bx0, bx1, bz0, bz1) = (x0 - 1, x1 + 1, z0 - 1, z1 + 1);
    let mut exterior: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut stack = vec![(bx0, bz0)];
    while let Some((x, z)) = stack.pop() {
        if x < bx0 || x > bx1 || z < bz0 || z > bz1 {
            continue;
        }
        if exterior.contains(&(x, z)) || walls.contains(&(x, z)) {
            continue;
        }
        exterior.insert((x, z));
        stack.extend([(x - 1, z), (x + 1, z), (x, z - 1), (x, z + 1)]);
    }
    // Remaining non-wall cells inside the bbox are rooms; number the
    // connected components in scan order.
    let mut room_count = 0;
    for z in z0..=z1 {
        for x in x0..=x1 {
            let c = (x, z);
            if walls.contains(&c) || exterior.contains(&c) || out.contains_key(&c) {
                continue;
            }
            let id = room_count;
            room_count += 1;
            let mut stack = vec![c];
            while let Some(cc) = stack.pop() {
                if cc.0 < x0 || cc.0 > x1 || cc.1 < z0 || cc.1 > z1 {
                    continue;
                }
                if walls.contains(&cc) || exterior.contains(&cc) || out.contains_key(&cc) {
                    continue;
                }
                out.insert(cc, id);
                stack.extend([(cc.0 - 1, cc.1), (cc.0 + 1, cc.1), (cc.0, cc.1 - 1), (cc.0, cc.1 + 1)]);
            }
        }
    }
    (out, room_count)
}

/// Bake a graph into colour-grouped, cut-away-ordered parts. `seed`
/// selects the building's room palette rotation (deterministic — every
/// peer bakes identical colours from the same template + seed).
pub fn bake_walls(g: &WallGraph, seed: u32) -> WallBake {
    let (rooms, _count) = rooms_of(g);
    let resolve = move |centroid: [f32; 3], normal: [f32; 3], material: [f32; 3]| -> [f32; 3] {
        if normal[1].abs() > 0.5 {
            return material; // tops, sills, lintels stay material
        }
        // Sample the space this face looks into. 60 units reaches the
        // adjacent CELL for every face geometry: an outer-hugging
        // perimeter wall's interior face sits 56 units from the room
        // boundary (40 − (−16)), a centred divider face 28, an outer
        // face 0 — and 60 stays well inside the 80-unit neighbour.
        let sx = centroid[0] + normal[0] * 60.0;
        let sz = centroid[2] + normal[2] * 60.0;
        let cell = ((sx / cdda::CDDA_TILE).floor() as i32, (sz / cdda::CDDA_TILE).floor() as i32);
        match rooms.get(&cell) {
            Some(room) => ROOM_PALETTE[(seed as usize + room * 3) % ROOM_PALETTE.len()],
            None => material, // faces the exterior (or another wall)
        }
    };
    let groups = crate::wall_mesh::wall_graph_mesh_with(g, &resolve);
    // Cut-away ordering: far triangles first, camera-side (near)
    // after. The iso camera looks from +x,+z, so geometry POSITIONED
    // past the building centre toward the camera is what stands
    // between the camera and the interior — classified by triangle
    // centroid x+z (template-local; the anchor is the centre), NOT by
    // face normal: a far wall's interior face points at the camera
    // but must stay visible as the backdrop. Mirrors the cube path's
    // old (pos.x + pos.z > depth + 40) rule, statically.
    // FOR NOW: a rotating camera would invalidate this bake.
    let parts = groups
        .into_iter()
        .map(|(color, verts, indices)| {
            let mut far: Vec<u32> = Vec::new();
            let mut near: Vec<(f32, [u32; 3])> = Vec::new();
            for t in indices.chunks(3) {
                let c = t.iter().fold(0.0f32, |acc, &i| {
                    let p = verts[i as usize].pos;
                    acc + (p[0] + p[2]) / 3.0
                });
                if c > cdda::CDDA_TILE * 0.5 {
                    near.push((c, [t[0], t[1], t[2]]));
                } else {
                    far.extend_from_slice(t);
                }
            }
            // Near triangles depth-ascending, so the draw range can
            // stop exactly at the player's depth (cube-path parity:
            // hide only walls IN FRONT of the player, keep the rest).
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            let near_start = far.len();
            let near_depths: Vec<f32> = near.iter().map(|(d, _)| *d).collect();
            for (_, t) in &near {
                far.extend_from_slice(t);
            }
            let ghost_indices: Vec<u32> =
                near.iter().rev().flat_map(|(_, t)| t.iter().copied()).collect();
            WallPart { color, verts, indices: far, near_start, near_depths, ghost_indices }
        })
        .collect();
    WallBake { parts }
}

/// A baked building placed in the world: anchor position plus its bake.
pub struct PlacedWalls {
    /// Chunk coordinates the building's anchor lives in — the cache key
    /// the browser path re-bakes on (player chunk crossings).
    pub key: (i32, i32),
    pub anchor: Vec3,
    pub bake: WallBake,
}

/// Is the player inside THIS building? Same roof-overlap test the
/// cube cut-away uses: standing under a roof slab, and that overhead
/// cell lies within this building's footprint radius. When true, the
/// draw uses `0..near_start` — the camera-facing walls drop.
pub fn player_inside(anchor: Vec3, snap: &crate::scene::SceneSnapshot) -> bool {
    let roof_half = cdda::CDDA_TILE / 2.0;
    let overhead = snap.structures.iter().find(|(p, k, _, _)| {
        *k == crate::template::PropKind::Roof
            && (p.x - snap.player.x).abs() <= roof_half
            && (p.z - snap.player.z).abs() <= roof_half
    });
    match overhead {
        Some((p, _, _, _)) => (p.x - anchor.x).abs() < 800.0 && (p.z - anchor.z).abs() < 800.0,
        None => false,
    }
}

/// Scan the chunks around `player` for stamped buildings and bake each
/// one's (rotated) wall graph. Pure — same inputs, same bakes, on every
/// peer and every path (native seer render + browser frame).
pub fn visible_wall_bakes(
    player: Vec3,
    bt: &crate::buildings::BuildingTemplates,
    chunk_size: f32,
    radius_chunks: i32,
) -> Vec<PlacedWalls> {
    let num = bt.templates.len();
    if num == 0 {
        return Vec::new();
    }
    let pcx = (player.x / chunk_size).floor() as i32;
    let pcz = (player.z / chunk_size).floor() as i32;
    let mut out = Vec::new();
    for cz in (pcz - radius_chunks)..=(pcz + radius_chunks) {
        for cx in (pcx - radius_chunks)..=(pcx + radius_chunks) {
            let Some(anchor) = cdda::building_anchor_in_chunk(cx, cz, chunk_size) else {
                continue;
            };
            let idx = cdda::building_index(cx, cz, num);
            let rot = cdda::building_rotation(cx, cz);
            let t = cdda::rotate_template(&bt.templates[idx], rot);
            if t.walls.nodes.is_empty() {
                continue;
            }
            // Seed from the anchor cell so two buildings of the same
            // template still colour their rooms differently.
            let seed = crate::hash::wang_hash(cx, cz, 0xB11D);
            out.push(PlacedWalls { key: (cx, cz), anchor, bake: bake_walls(&t.walls, seed) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p_shape_graph() -> WallGraph {
        let json = r#"[{
            "om_terrain": "p_shape",
            "object": {
                "rows": [
                    "wwwww",
                    "o d w",
                    "wdwdw",
                    "  w w",
                    "  w o",
                    "  www"
                ],
                "terrain": {
                    "w": "t_wall",
                    "o": "t_window",
                    "d": "t_door_c"
                }
            }
        }]"#;
        cdda::mapgen_to_template(json, "p_shape", cdda::CDDA_TILE, 0)
            .unwrap()
            .walls
    }

    #[test]
    fn p_shape_has_three_rooms() {
        // Two door-split rooms in the top box + one in the stem — the
        // same count the graph's cyclomatic number pinned in slice 1.
        let (rooms, count) = rooms_of(&p_shape_graph());
        assert_eq!(count, 3, "room components");
        // Every room cell maps to an id below the count.
        assert!(rooms.values().all(|&r| r < count));
        // The top-left room contains cell (1, 1) — row 1, col 1 in the
        // fixture, cell coords relative to the 5×6 grid centring.
        // Node offsets are (col−2.5)·80, (row−3)·80 → cell (col−2.5+ε)…
        // easier: just assert the three rooms have disjoint non-empty
        // cell sets.
        let mut sizes = [0usize; 3];
        for &r in rooms.values() {
            sizes[r] += 1;
        }
        assert!(sizes.iter().all(|&s| s > 0), "every room has cells: {sizes:?}");
    }

    #[test]
    fn interior_faces_take_room_colors_and_exterior_keeps_material() {
        let g = p_shape_graph();
        let bake = bake_walls(&g, 7);
        assert!(!bake.parts.is_empty());
        // The material colour of the walls (t_wall → generic wall colour
        // via cells.rs) appears on exterior-facing geometry…
        let material = g
            .nodes
            .iter()
            .find_map(|n| n.color)
            .expect("walls carry a material colour");
        let has_material_part = bake
            .parts
            .iter()
            .any(|p| (p.color[0] - material[0]).abs() < 1e-3);
        assert!(has_material_part, "exterior material colour part missing");
        // …and at least two distinct room colours exist beyond it (the
        // P-shape has 3 rooms; at least 2 must be visible as parts).
        let room_parts = bake
            .parts
            .iter()
            .filter(|p| (p.color[0] - material[0]).abs() > 1e-3)
            .count();
        assert!(room_parts >= 2, "expected ≥2 room-coloured parts, got {room_parts}");
    }

    #[test]
    fn room_colors_are_deterministic_and_vary_with_seed() {
        let g = p_shape_graph();
        let a = bake_walls(&g, 1);
        let b = bake_walls(&g, 1);
        let colors = |bk: &WallBake| -> Vec<[u32; 3]> {
            bk.parts
                .iter()
                .map(|p| [p.color[0].to_bits(), p.color[1].to_bits(), p.color[2].to_bits()])
                .collect()
        };
        assert_eq!(colors(&a), colors(&b), "same seed → same colours");
        let c = bake_walls(&g, 2);
        assert_ne!(colors(&a), colors(&c), "different seed → different palette rotation");
    }

    #[test]
    fn rotated_corpus_house_still_finds_rooms() {
        // Rotation safety on real authored data: a rotated house_01
        // still derives rooms and colours them — the graph, the cell
        // mapping and the face sampling all rotate coherently.
        let t = cdda::house_template().expect("house_01 imports");
        for rot in 0..4u8 {
            let g = cdda::rotate_template(&t, rot).walls;
            let (_, count) = rooms_of(&g);
            assert!(count >= 2, "rotation {rot}: expected ≥2 rooms, got {count}");
            let material = g.nodes.iter().find_map(|n| n.color).unwrap();
            let bake = bake_walls(&g, 3);
            let room_parts = bake
                .parts
                .iter()
                .filter(|p| (p.color[0] - material[0]).abs() > 1e-3)
                .count();
            assert!(room_parts >= 2, "rotation {rot}: room-coloured parts missing");
        }
    }

    #[test]
    fn ghost_indices_complement_the_opaque_draw_exactly() {
        // Ghost = exactly what the opaque draw skipped, never both —
        // slice 5's invariant, by construction: near triangles sorted
        // depth-ASCENDING in `indices[near_start..]` for the opaque
        // draw, and the SAME triangles depth-DESCENDING in
        // `ghost_indices` so the cut set is always a prefix (the web
        // crossing can only draw prefixes). For every possible cut
        // depth k, opaque's k nearest-visible + ghost's (N−k) prefix
        // must partition the near set.
        let bake = bake_walls(&p_shape_graph(), 0);
        for p in &bake.parts {
            let near: Vec<&[u32]> = p.indices[p.near_start..].chunks(3).collect();
            let ghost: Vec<&[u32]> = p.ghost_indices.chunks(3).collect();
            assert_eq!(near.len(), ghost.len(), "ghost covers the whole near set");
            let n = near.len();
            for k in 0..=n {
                // Opaque draws near[..k]; ghost draws ghost[..n-k].
                let mut opaque: Vec<&[u32]> = near[..k].to_vec();
                let mut ghosted: Vec<&[u32]> = ghost[..n - k].to_vec();
                opaque.sort();
                ghosted.sort();
                let mut union = opaque.clone();
                union.extend(ghosted.iter());
                union.sort();
                union.dedup();
                assert_eq!(
                    union.len(),
                    n,
                    "opaque[..{k}] and ghost[..{}] must partition the near set",
                    n - k
                );
            }
            // Ghost ordering is depth-descending.
            let depth = |t: &[u32]| -> f32 {
                t.iter().fold(0.0f32, |acc, &i| {
                    let v = p.verts[i as usize].pos;
                    acc + (v[0] + v[2]) / 3.0
                })
            };
            for w in ghost.windows(2) {
                assert!(depth(w[0]) >= depth(w[1]) - 1e-3, "ghost not depth-descending");
            }
        }
    }

    #[test]
    fn near_geometry_sorts_after_far_for_the_cutaway_range() {
        let bake = bake_walls(&p_shape_graph(), 0);
        for p in &bake.parts {
            for (i, t) in p.indices.chunks(3).enumerate() {
                let c = t.iter().fold(0.0f32, |acc, &ix| {
                    let v = p.verts[ix as usize].pos;
                    acc + (v[0] + v[2]) / 3.0
                });
                let near = c > cdda::CDDA_TILE * 0.5;
                if i * 3 < p.near_start {
                    assert!(!near, "near triangle before near_start in part");
                } else {
                    assert!(near, "far triangle after near_start in part");
                }
            }
            // The union of both ranges is the whole part.
            assert_eq!(p.indices.len() % 3, 0);
        }
        // Some geometry is near, some far — both ranges are real.
        assert!(bake.parts.iter().any(|p| p.near_start > 0));
        assert!(bake.parts.iter().any(|p| p.near_start < p.indices.len()));
    }
}
