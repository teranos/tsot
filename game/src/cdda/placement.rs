//! The heart of the importer: mapgen JSON → `Template` with edge-placed
//! walls, plus the separate roof pass.
//!
//! Placement rules (from the user's drawings):
//! - Flood-fill from the mapgen boundary through non-wall-line cells
//!   marks the exterior. The interior is what's left over.
//! - Each wall/window cell participates in a horizontal run (if E or W
//!   is wall-line) and/or a vertical run (if N or S is wall-line).
//! - For each active run, the "interior side" is any perpendicular
//!   direction where any cell of the run touches interior.
//! - Emit the run's segment offset toward its interior side (by
//!   `tile_size/2`), or centred for a divider with interior on both
//!   perp sides (convention: shift +z / +x — "always positive").
//! - Corner cells participate in BOTH runs → emit two segments meeting
//!   at the interior-facing inner corner.
//! - Isolated pillars (no wall neighbours) or fully-enclosed
//!   junctions (interior nowhere in perp) fall back to centred full-tile.

use std::collections::HashMap;
use std::fmt;

use bevy_math::Vec3;

use crate::template::{Prop, PropKind, Template};

use super::cells::{cell_to_prop, is_wall_line_char};
use super::parse::{Entry, first_id, om_matches};

/// World units per CDDA tile. Wall props are sized to this (see the
/// Wall collider in `template.rs` and the Wall appearance in `scene.rs`,
/// both 80) so the grid tiles seamlessly.
pub const CDDA_TILE: f32 = 80.0;

/// Roof elevation — matches the wall height so the slab caps the walls.
pub const ROOF_HEIGHT: f32 = 220.0;

/// Import failures — surfaced (sacred), never swallowed.
#[derive(Debug, PartialEq, Eq)]
pub enum CddaError {
    Parse(String),
    NotFound(String),
    NoObject(String),
}

impl fmt::Display for CddaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CddaError::Parse(m) => write!(f, "CDDA mapgen parse error: {m}"),
            CddaError::NotFound(n) => write!(f, "CDDA mapgen '{n}' not found in file"),
            CddaError::NoObject(n) => write!(f, "CDDA mapgen '{n}' has no object"),
        }
    }
}

/// Parse a CDDA mapgen JSON file, select the entry named `om_terrain`,
/// and build a centred `Template` (col → +x, row → +z, grid centred on
/// the anchor).
pub fn mapgen_to_template(
    json: &str,
    om_terrain: &str,
    tile_size: f32,
    seed: u32,
) -> Result<Template, CddaError> {
    let entries: Vec<Entry> =
        serde_json::from_str(json).map_err(|e| CddaError::Parse(e.to_string()))?;
    let entry = entries
        .iter()
        .find(|e| om_matches(&e.om_terrain, om_terrain))
        .ok_or_else(|| CddaError::NotFound(om_terrain.to_string()))?;
    let obj = entry
        .object
        .as_ref()
        .ok_or_else(|| CddaError::NoObject(om_terrain.to_string()))?;
    // Resolve referenced palettes into a flat char→id terrain map, then
    // overlay the inline terrain (inline overrides the palettes). The
    // palette's furniture map is dropped: no furniture is spawned yet.
    let mut terrain = if obj.palettes.is_empty() {
        HashMap::new()
    } else {
        crate::palette::resolve(&obj.palettes, seed).0
    };
    for (sym, val) in &obj.terrain {
        if let (Some(ch), Some(id)) = (sym.chars().next(), first_id(val)) {
            terrain.insert(ch, id.to_string());
        }
    }

    let height = obj.rows.len();
    let width = obj.rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);

    // Pass 1: base (kind, colour) per cell (walls are the plain Wall kind).
    type Cell = Option<(PropKind, Option<[f32; 3]>)>;
    let grid: Vec<Vec<Cell>> = obj
        .rows
        .iter()
        .map(|row| {
            let mut cells: Vec<_> = row
                .chars()
                .map(|ch| cell_to_prop(ch, &terrain))
                .collect();
            cells.resize(width, None);
            cells
        })
        .collect();

    // The wall LINE: cells whose char resolves to wall/fence/window/door
    // /gate terrain — the connective tissue of a building's outline.
    // Doors and gates don't emit a prop yet, but they still block flood-
    // fill and count as run cells, so the wall line is unbroken across
    // them and the interior stays interior.
    let mut wall_line = vec![vec![false; width]; height];
    for (r, row) in obj.rows.iter().enumerate() {
        for (c, ch) in row.chars().enumerate() {
            if c >= width {
                break;
            }
            if is_wall_line_char(ch, &terrain) {
                wall_line[r][c] = true;
            }
        }
    }
    let is_solid = |r: i32, c: i32| -> bool {
        r >= 0
            && c >= 0
            && (r as usize) < height
            && (c as usize) < width
            && wall_line[r as usize][c as usize]
    };

    // Flood-fill exterior — every non-solid cell reachable from the
    // mapgen boundary. Cells not reached and not solid are "interior"
    // (a floor of a fully enclosed room).
    let mut exterior = vec![vec![false; width]; height];
    let mut stack: Vec<(i32, i32)> = Vec::new();
    for r in 0..height {
        for c in 0..width {
            let on_boundary = r == 0 || c == 0 || r + 1 == height || c + 1 == width;
            if on_boundary && !is_solid(r as i32, c as i32) {
                stack.push((r as i32, c as i32));
            }
        }
    }
    while let Some((r, c)) = stack.pop() {
        if r < 0 || c < 0 || (r as usize) >= height || (c as usize) >= width {
            continue;
        }
        let (ru, cu) = (r as usize, c as usize);
        if exterior[ru][cu] || is_solid(r, c) {
            continue;
        }
        exterior[ru][cu] = true;
        for (dr, dc) in [(-1i32, 0), (1, 0), (0, -1), (0, 1)] {
            stack.push((r + dr, c + dc));
        }
    }
    let is_interior = |r: i32, c: i32| -> bool {
        if r < 0 || c < 0 || (r as usize) >= height || (c as usize) >= width {
            return false;
        }
        !exterior[r as usize][c as usize] && !is_solid(r, c)
    };

    // Per-cell interior sides. Previously aggregated over the whole run,
    // which mislabelled the bottom-room's west wall as a "divider" just
    // because one cell of the same column was between two rooms — a
    // single wall column can be a perimeter for one room and a divider
    // between others.
    let horiz_at = |r: i32, c: i32| -> (bool, bool) {
        (is_interior(r - 1, c), is_interior(r + 1, c))
    };
    let vert_at = |r: i32, c: i32| -> (bool, bool) {
        (is_interior(r, c + 1), is_interior(r, c - 1))
    };
    // Run-scope aligns emission side for cells that have no direct
    // interior clue on their own — a T-junction wall cell whose direct
    // neighbours are all wall_line still needs to render along the same
    // outer edge as the rest of its horizontal / vertical run. NOT used
    // to classify dividers (that's per-cell); used only to determine
    // WHICH edge (N vs S, E vs W) an already-detected wall sits on.
    let is_ext_check = |rr: i32, cc: i32| -> bool {
        if rr < 0 || cc < 0 || (rr as usize) >= height || (cc as usize) >= width {
            return true;
        }
        exterior[rr as usize][cc as usize]
    };
    let horiz_run_side = |r: i32, c: i32| -> (bool, bool) {
        let (mut ext_n, mut ext_s) = (false, false);
        let mut cc = c;
        while is_solid(r, cc) {
            if is_ext_check(r - 1, cc) { ext_n = true; }
            if is_ext_check(r + 1, cc) { ext_s = true; }
            cc -= 1;
        }
        let mut cc = c + 1;
        while is_solid(r, cc) {
            if is_ext_check(r - 1, cc) { ext_n = true; }
            if is_ext_check(r + 1, cc) { ext_s = true; }
            cc += 1;
        }
        (ext_n, ext_s)
    };
    let vert_run_side = |r: i32, c: i32| -> (bool, bool) {
        let (mut ext_e, mut ext_w) = (false, false);
        let mut rr = r;
        while is_solid(rr, c) {
            if is_ext_check(rr, c + 1) { ext_e = true; }
            if is_ext_check(rr, c - 1) { ext_w = true; }
            rr -= 1;
        }
        let mut rr = r + 1;
        while is_solid(rr, c) {
            if is_ext_check(rr, c + 1) { ext_e = true; }
            if is_ext_check(rr, c - 1) { ext_w = true; }
            rr += 1;
        }
        (ext_e, ext_w)
    };

    let ew_kind = |base: PropKind| match base {
        PropKind::Wall => PropKind::WallEW,
        PropKind::Window => PropKind::WindowEW,
        other => other,
    };
    let ns_kind = |base: PropKind| match base {
        PropKind::Wall => PropKind::WallNS,
        PropKind::Window => PropKind::WindowNS,
        other => other,
    };

    // Pass 2: place each cell. Walls/windows follow the edge-placement
    // rules; every other prop stays centred on its tile. Wall geometry
    // sits ENTIRELY within its own wall cell (space-maximising for the
    // adjacent floor cell) — the wall's inner face is exactly on the
    // grid boundary; the wall extends `WALL_HALF_THICKNESS` back into
    // the wall cell, never into the room.
    let cx = width as f32 / 2.0;
    let cz = height as f32 / 2.0;
    let half = tile_size / 2.0;
    // Half of a wall's thin-axis extent (WallEW/WallNS are 24 wide in
    // prop_appearance → 12 half). Kept local since prop_appearance
    // lives in scene.rs; if the wall thickness ever changes there,
    // update this too.
    const WALL_HALF_THICKNESS: f32 = 12.0;
    let mut props = Vec::new();
    for (r_idx, row_cells) in grid.iter().enumerate() {
        for (c_idx, cell) in row_cells.iter().enumerate() {
            let Some((base, color)) = *cell else { continue };
            let r = r_idx as i32;
            let c = c_idx as i32;
            let cx_world = (c_idx as f32 - cx) * tile_size;
            let cz_world = (r_idx as f32 - cz) * tile_size;
            let mut emit = |x: f32, z: f32, kind: PropKind| {
                let offset = Vec3::new(x, 0.0, z);
                props.push(match color {
                    Some(col) => Prop::colored(offset, kind, col),
                    None => Prop::at(offset, kind),
                });
            };

            let is_wall_or_window = matches!(base, PropKind::Wall | PropKind::Window);
            if !is_wall_or_window {
                emit(cx_world, cz_world, base);
                continue;
            }

            // Unified rule (no full-tile blocks anywhere):
            //   1. For each of 4 sides, if the direct neighbour is
            //      EXTERIOR (reachable by flood-fill from mapgen
            //      boundary), emit a thin wall on that outer edge —
            //      that's a perimeter wall.
            //   2. Divider: if a wall cell has interior on both perp
            //      direct neighbours (rooms on both sides), emit walls
            //      on both perp outer edges — one wall facing each
            //      room. No arbitrary +positive convention needed.
            //   3. Diagonal corner: if no direct exterior and no direct
            //      divider case, but a diagonal is interior, emit walls
            //      on the two outer edges opposite that diagonal
            //      (interior at SE → walls on N + W). Multiple diagonals
            //      interior → union of their edge pairs (T-junction
            //      cells can end up with walls on 3 or 4 sides).
            let (hn, hs) = horiz_at(r, c);
            let (ve, vw) = vert_at(r, c);
            let is_ext = |rr: i32, cc: i32| -> bool {
                if rr < 0 || cc < 0 || (rr as usize) >= height || (cc as usize) >= width {
                    return true; // OOB counts as exterior for perimeter emission
                }
                exterior[rr as usize][cc as usize]
            };
            let ext_n = is_ext(r - 1, c);
            let ext_s = is_ext(r + 1, c);
            let ext_e = is_ext(r, c + 1);
            let ext_w = is_ext(r, c - 1);

            let mut emit_n = ext_n;
            let mut emit_s = ext_s;
            let mut emit_e = ext_e;
            let mut emit_w = ext_w;

            // Divider: perp direct interiors on both sides. Neither side
            // is "outer" (both are rooms), so emitting on both faces would
            // put two parallel walls in the same cell with a 56-unit gap
            // — the "double wall" visual. Emit ONE wall, centred in the
            // cell, same 24-unit thickness as any other wall.
            let divider_ew = hn && hs;
            let divider_ns = ve && vw;

            // Corner / T-junction: diagonal interiors imply walls on
            // outer edges opposite that diagonal (interior at SE → walls
            // on N + W). Gated on the direct neighbour on that side NOT
            // being a wall_line cell — otherwise the "wall" would sit
            // inside another wall (invisible, and over-counts the
            // geometry when the neighbouring cell already emits there).
            let d_nw = is_interior(r - 1, c - 1);
            let d_ne = is_interior(r - 1, c + 1);
            let d_sw = is_interior(r + 1, c - 1);
            let d_se = is_interior(r + 1, c + 1);
            let n_open = !is_solid(r - 1, c);
            let s_open = !is_solid(r + 1, c);
            let e_open = !is_solid(r, c + 1);
            let w_open = !is_solid(r, c - 1);
            if d_se {
                emit_n = emit_n || n_open;
                emit_w = emit_w || w_open;
            }
            if d_sw {
                emit_n = emit_n || n_open;
                emit_e = emit_e || e_open;
            }
            if d_ne {
                emit_s = emit_s || s_open;
                emit_w = emit_w || w_open;
            }
            if d_nw {
                emit_s = emit_s || s_open;
                emit_e = emit_e || e_open;
            }

            // If per-cell decided no horizontal wall but this cell IS
            // part of a horizontal run, align to the run's outer edge
            // (any cell in the run with a direct exterior on N or S).
            // Same for vertical. Fixes T-junction cells whose direct
            // neighbours are all wall_line so the per-cell rule finds
            // no orientation, but the RUN through them has a clear
            // edge from other cells.
            if !emit_n && !emit_s && (is_solid(r, c - 1) || is_solid(r, c + 1)) {
                let (run_n, run_s) = horiz_run_side(r, c);
                emit_n = run_n;
                emit_s = run_s;
            }
            if !emit_e && !emit_w && (is_solid(r - 1, c) || is_solid(r + 1, c)) {
                let (run_e, run_w) = vert_run_side(r, c);
                emit_e = run_e;
                emit_w = run_w;
            }

            if emit_n {
                emit(cx_world, cz_world - half + WALL_HALF_THICKNESS, ew_kind(base));
            }
            if emit_s {
                emit(cx_world, cz_world + half - WALL_HALF_THICKNESS, ew_kind(base));
            }
            if emit_e {
                emit(cx_world + half - WALL_HALF_THICKNESS, cz_world, ns_kind(base));
            }
            if emit_w {
                emit(cx_world - half + WALL_HALF_THICKNESS, cz_world, ns_kind(base));
            }
            // Divider case: single centred wall (no ±half shift), one
            // per orientation. A cell that's a divider on BOTH axes (a
            // wall in the middle of a 4-room cross) gets both.
            if divider_ew {
                emit(cx_world, cz_world, ew_kind(base));
            }
            if divider_ns {
                emit(cx_world, cz_world, ns_kind(base));
            }
            // No neighbours at all matched — isolated wall cell.
            // Emit a centered thin bar rather than a block.
            if !emit_n && !emit_s && !emit_e && !emit_w && !divider_ew && !divider_ns {
                emit(cx_world, cz_world, ew_kind(base));
            }
        }
    }
    Ok(Template { props })
}

/// Import a roof z-level by OCCUPANCY — every non-blank cell becomes a
/// flat roof slab at `height_y`. Roofs are palette-driven in CDDA, but
/// a flat roof is visually uniform, so we skip the palette resolver:
/// "cell is not blank" == "roofed here".
pub fn roof_to_props(
    json: &str,
    om_terrain: &str,
    tile_size: f32,
    height_y: f32,
) -> Result<Vec<Prop>, CddaError> {
    let entries: Vec<Entry> =
        serde_json::from_str(json).map_err(|e| CddaError::Parse(e.to_string()))?;
    let entry = entries
        .iter()
        .find(|e| om_matches(&e.om_terrain, om_terrain))
        .ok_or_else(|| CddaError::NotFound(om_terrain.to_string()))?;
    let obj = entry
        .object
        .as_ref()
        .ok_or_else(|| CddaError::NoObject(om_terrain.to_string()))?;
    let height = obj.rows.len();
    let width = obj.rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);
    let cx = width as f32 / 2.0;
    let cz = height as f32 / 2.0;
    let mut props = Vec::new();
    for (r, row) in obj.rows.iter().enumerate() {
        for (c, ch) in row.chars().enumerate() {
            if ch != ' ' {
                let x = (c as f32 - cx) * tile_size;
                let z = (r as f32 - cz) * tile_size;
                props.push(Prop::at(Vec3::new(x, height_y, z), PropKind::Roof));
            }
        }
    }
    Ok(props)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_mapgen(rows: &[&str]) -> String {
        let joined = rows
            .iter()
            .map(|r| format!("{r:?}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"[{{"om_terrain":"tt","object":{{"rows":[{joined}],"terrain":{{"w":"t_wall",".":"t_floor",":":"t_window"}}}}}}]"#
        )
    }

    #[test]
    fn perimeter_wall_hugs_the_exterior_edge_of_its_cell() {
        // 3×3 room: solid perimeter, one floor at (1, 1). Each wall cell
        // carries its wall on its OUTWARD-facing boundary — the wall
        // traces the building's outside outline, inner face on the grid
        // line so it hugs the exterior and the interior floor stays
        // maximal. The top-middle cell (0, 1) faces exterior to the NORTH
        // (its south neighbour is the floor), so it emits one WallEW on
        // its north outer edge.
        let json = synthetic_mapgen(&["www", "w.w", "www"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Cell (0, 1) centre: x = (1 − 1.5) × 80 = −40, z = (0 − 1.5) × 80 = −120.
        // North outer edge: z = −160. Wall centre = edge + WALL_HALF_THICKNESS
        // (12), sitting entirely inside the cell → z = −148.
        let has_outer = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallEW)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-148.0)).abs() < 1e-3
        });
        assert!(
            has_outer,
            "top-middle wall should hug its north outer edge (z = north_edge + 12 = −148); got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn corner_cell_emits_both_outward_faces() {
        // Each wall cell carries its wall on its outward-facing edges, so
        // a corner cell — facing exterior on two sides — emits two
        // segments: the L that turns the building's outline at the
        // corner. NW corner (0, 0) faces north and west.
        let json = synthetic_mapgen(&["www", "w.w", "www"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Cell (0, 0) centre: x = −120, z = −120. North outer edge z = −160
        // → WallEW centre z = −148. West outer edge x = −160 → WallNS
        // centre x = −148.
        let has_north = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallEW)
                && (p.offset.x - (-120.0)).abs() < 1e-3
                && (p.offset.z - (-148.0)).abs() < 1e-3
        });
        let has_west = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-148.0)).abs() < 1e-3
                && (p.offset.z - (-120.0)).abs() < 1e-3
        });
        assert!(
            has_north && has_west,
            "NW corner should emit its north + west outer faces; got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
        // Outer outline of a 3×3 room: 4 corners × 2 faces + 4 edge-middles
        // × 1 face = 12 segments.
        let count = t
            .props
            .iter()
            .filter(|p| matches!(p.kind, PropKind::WallEW | PropKind::WallNS))
            .count();
        assert_eq!(count, 12, "expected 12 outer-outline segments, got {count}");
    }

    #[test]
    fn divider_between_two_rooms_is_a_single_centred_wall() {
        // 3×5 mapgen: two 1×1 rooms separated by a divider at (1, 2). A
        // divider has interior on BOTH perpendicular sides (rooms east
        // AND west), so neither side is "outer". Emitting on both outer
        // edges would give two parallel walls with a 56-unit gap — the
        // "double wall" visual. The rule: one WallNS, centred in the
        // cell (no ±half shift), same 24-unit thickness.
        let json = synthetic_mapgen(&["wwwww", "w.w.w", "wwwww"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Cell (1, 2) centre: x = (2 − 2.5) × 80 = −40, z = (1 − 1.5) × 80 = −40.
        let divider_walls: Vec<_> = t
            .props
            .iter()
            .filter(|p| {
                matches!(p.kind, PropKind::WallNS)
                    && (p.offset.z - (-40.0)).abs() < 1e-3
                    && p.offset.x > -80.0
                    && p.offset.x < 0.0
            })
            .collect();
        assert_eq!(
            divider_walls.len(),
            1,
            "divider should be a SINGLE centred wall, not one per face; got: {:?}",
            divider_walls
        );
        let d = divider_walls[0];
        assert!(
            (d.offset.x - (-40.0)).abs() < 1e-3,
            "divider centred in its cell (x = −40); got x = {}",
            d.offset.x
        );
    }

    #[test]
    fn windows_use_the_same_placement_rules_as_walls() {
        // A window cell obeys the same outer-edge rule as a wall cell.
        // The window at (0, 1) faces exterior north → WindowEW on the
        // north outer edge (z = −148), exactly where a WallEW would sit.
        let json = synthetic_mapgen(&["w:w", "w.w", "www"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        let has_window = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WindowEW)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-148.0)).abs() < 1e-3
        });
        assert!(
            has_window,
            "window should follow the outer-edge rule (north outer edge, z = −148); got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn t_junction_divider_doesnt_protrude_into_the_perimeter_cell() {
        // 4×5: two 1×1 rooms separated by a divider at col 2. Perimeter
        // walls all around. The T-junction cells (0, 2) and (3, 2) are
        // where the divider meets the top and bottom perimeter — the
        // vertical divider should NOT emit at those cells.
        let json = synthetic_mapgen(&["wwwww", "w.w.w", "w.w.w", "wwwww"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // For 4×5 with tile=80: cx=2.5, cz=2.0.
        // Cell (0, 2) centre: x=−40, z=−160. Vert emission (post space-
        // max shift) would sit at x = 0 − 12 = −12, z = −160. None.
        let has_top_protrusion = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-12.0)).abs() < 1e-3
                && (p.offset.z - (-160.0)).abs() < 1e-3
        });
        assert!(
            !has_top_protrusion,
            "top T-junction should not emit a vertical divider at (0, 2); got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
        // Same for the bottom T-junction (row 3, z = 80).
        let has_bottom_protrusion = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-12.0)).abs() < 1e-3
                && (p.offset.z - 80.0).abs() < 1e-3
        });
        assert!(!has_bottom_protrusion, "bottom T-junction should not emit vertical");
        // But the divider IS emitted at the middle cells (1, 2) and (2, 2).
        let mid_1 = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-12.0)).abs() < 1e-3
                && (p.offset.z - (-80.0)).abs() < 1e-3
        });
        let mid_2 = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-12.0)).abs() < 1e-3
                && p.offset.z.abs() < 1e-3
        });
        assert!(mid_1 && mid_2, "the divider still emits at rows 1 and 2");
    }

    /// User's typed spec, verbatim. Runs the current importer and
    /// dumps every emitted prop for user review — no assertions.
    ///
    ///   Row 0: wwwww
    ///   Row 1: o d w
    ///   Row 2: wdwdw
    ///   Row 3:   w w
    ///   Row 4:   w o
    ///   Row 5:   www
    ///
    /// w=wall, o=window, d=door, space=floor/exterior.
    #[test]
    #[ignore = "dump-only for user review; run with `cargo test -- --ignored`"]
    fn dump_user_p_shape_building() {
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
        let t = mapgen_to_template(json, "p_shape", CDDA_TILE, 0).unwrap();
        let mut lines = Vec::new();
        lines.push(format!("=== {} props emitted ===", t.props.len()));
        lines.push(String::from(
            "cx=2.5 cz=3.0 tile=80 → col_c = (c-2.5)*80, row_r = (r-3)*80",
        ));
        lines.push(String::from(
            "col centres:  0=-200  1=-120  2=-40  3=40  4=120",
        ));
        lines.push(String::from(
            "row centres:  0=-240  1=-160  2=-80  3=0    4=80   5=160",
        ));
        lines.push(String::new());
        for p in &t.props {
            lines.push(format!(
                "  {:?} at x={:>7.1} z={:>7.1}",
                p.kind, p.offset.x, p.offset.z
            ));
        }
        panic!("\n{}\n", lines.join("\n"));
    }

    #[test]
    fn unknown_om_terrain_is_a_surfaced_error() {
        let json = synthetic_mapgen(&["www", "w.w", "www"]);
        let err = mapgen_to_template(&json, "s_no_such_building", CDDA_TILE, 0).unwrap_err();
        assert_eq!(err, CddaError::NotFound("s_no_such_building".to_string()));
    }
}
