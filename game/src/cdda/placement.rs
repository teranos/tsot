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

            // Fence: emit one centred prop per cell, oriented by whether
            // its neighbours (horizontal vs vertical) are also fences.
            // Fences don't participate in the wall edge-placement rules
            // — they're a yard boundary, not a room seal (excluded from
            // is_wall_line_char so flood-fill passes through).
            if matches!(base, PropKind::Fence) {
                let is_fence_at = |rr: i32, cc: i32| -> bool {
                    if rr < 0 || cc < 0 || (rr as usize) >= height || (cc as usize) >= width {
                        return false;
                    }
                    matches!(grid[rr as usize][cc as usize], Some((PropKind::Fence, _)))
                };
                let horiz = is_fence_at(r, c - 1) || is_fence_at(r, c + 1);
                let vert = is_fence_at(r - 1, c) || is_fence_at(r + 1, c);
                let kind = match (horiz, vert) {
                    (true, false) => PropKind::FenceEW,
                    (false, true) => PropKind::FenceNS,
                    _ => PropKind::Fence, // isolated post or 4-way junction
                };
                emit(cx_world, cz_world, kind);
                continue;
            }

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

            // Divider takes precedence per axis: a single centred wall
            // serves both rooms, so the perimeter/diagonal emissions on
            // the same axis (which would give N-outer + S-outer + centre
            // = three parallel walls) must be suppressed.
            if divider_ew {
                emit_n = false;
                emit_s = false;
            }
            if divider_ns {
                emit_e = false;
                emit_w = false;
            }

            // If per-cell decided no horizontal wall but this cell IS
            // part of a horizontal run, align to the run's outer edge
            // (any cell in the run with a direct exterior on N or S).
            // Same for vertical. Fixes T-junction cells whose direct
            // neighbours are all wall_line so the per-cell rule finds
            // no orientation, but the RUN through them has a clear
            // edge from other cells.
            if !emit_n && !emit_s && !divider_ew && (is_solid(r, c - 1) || is_solid(r, c + 1)) {
                let (run_n, run_s) = horiz_run_side(r, c);
                emit_n = run_n;
                emit_s = run_s;
            }
            if !emit_e && !emit_w && !divider_ns && (is_solid(r - 1, c) || is_solid(r + 1, c)) {
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
            // T-junction stub: if a perpendicular divider is adjacent,
            // this cell's perimeter wall doesn't reach into the divider
            // (they sit on different planes — perimeter on the outer
            // edge, divider on the cell centre — leaving a ~56-unit
            // gap at the T). Emit a centred stub on the perpendicular
            // axis, spanning the cell, to bridge them. Different axis
            // from the perimeter, so the "one wall per axis per cell"
            // invariant is preserved.
            let neighbour_is_ns_divider = |rr: i32, cc: i32| -> bool {
                is_solid(rr, cc)
                    && is_interior(rr, cc + 1)
                    && is_interior(rr, cc - 1)
            };
            let neighbour_is_ew_divider = |rr: i32, cc: i32| -> bool {
                is_solid(rr, cc)
                    && is_interior(rr - 1, cc)
                    && is_interior(rr + 1, cc)
            };
            let need_ns_stub = !divider_ns
                && (neighbour_is_ns_divider(r - 1, c) || neighbour_is_ns_divider(r + 1, c));
            let need_ew_stub = !divider_ew
                && (neighbour_is_ew_divider(r, c - 1) || neighbour_is_ew_divider(r, c + 1));
            if need_ns_stub {
                emit(cx_world, cz_world, ns_kind(base));
            }
            if need_ew_stub {
                emit(cx_world, cz_world, ew_kind(base));
            }
            // No neighbours at all matched — isolated wall cell.
            // Emit a centered thin bar rather than a block.
            if !emit_n
                && !emit_s
                && !emit_e
                && !emit_w
                && !divider_ew
                && !divider_ns
                && !need_ns_stub
                && !need_ew_stub
            {
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
    fn fence_ring_leaves_the_enclosed_area_as_exterior() {
        // A fenced yard is bounded by fence chars but the interior
        // stays EXTERIOR — flood-fill must pass through fence cells,
        // otherwise the building walls facing the yard would be
        // detected as room-to-room dividers (the yard bug).
        // 5×5 mapgen with a fence ring around a single floor cell.
        // Fence chars use `f`; wall_line excludes fence, so the floor
        // at (2, 2) is reachable from the mapgen boundary.
        let json = r#"[{
            "om_terrain": "tt",
            "object": {
                "rows": ["fffff", "f...f", "f...f", "f...f", "fffff"],
                "terrain": {"f": "t_fence_barbed", ".": "t_floor"}
            }
        }]"#;
        let t = mapgen_to_template(json, "tt", CDDA_TILE, 0).unwrap();
        // 16 fence-perimeter cells (5+5+3+3 = 16) emit fence props.
        let fence_props = t.props.iter().filter(|p| {
            matches!(
                p.kind,
                PropKind::Fence | PropKind::FenceEW | PropKind::FenceNS
            )
        }).count();
        assert_eq!(fence_props, 16, "one fence prop per perimeter cell; got {fence_props}");
        // No wall props anywhere — this mapgen has no walls, only fence.
        // If flood-fill had failed to reach the interior, the fence
        // cells would have been treated as wall_line and interior
        // classification would have produced spurious wall geometry.
        let wall_props = t
            .props
            .iter()
            .filter(|p| matches!(p.kind, PropKind::Wall | PropKind::WallEW | PropKind::WallNS))
            .count();
        assert_eq!(wall_props, 0, "no walls should be emitted from a fence-only ring");
    }

    #[test]
    fn building_wall_adjacent_to_fenced_yard_emits_as_perimeter() {
        // Building (a single wall cell) sits inside a fenced yard.
        // Because fences don't block flood-fill, the yard stays
        // exterior — so the wall cell has exterior on all 4 sides and
        // emits outer perimeter walls, NOT centred dividers.
        // 5×5 mapgen with wall at (2, 2). Cx=Cz=2.5 → cell centre at
        // (−40, −40). Outer edges at cell centre ± (40−12) = ±28
        // relative to the centre → absolute (−68/−12) on each axis.
        let json = r#"[{
            "om_terrain": "tt",
            "object": {
                "rows": ["fffff", "f...f", "f.w.f", "f...f", "fffff"],
                "terrain": {"f": "t_fence_barbed", "w": "t_wall_log", ".": "t_floor"}
            }
        }]"#;
        let t = mapgen_to_template(json, "tt", CDDA_TILE, 0).unwrap();
        // No centred wall at the cell centre (that would be the divider
        // signature — proves yard was misclassified as interior).
        let has_centred = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallEW | PropKind::WallNS)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-40.0)).abs() < 1e-3
        });
        assert!(
            !has_centred,
            "yard-facing wall should NOT be a centred divider; got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
        // Four outer-edge walls: N at (−40, −68), S at (−40, −12),
        // E at (−12, −40), W at (−68, −40).
        let expected = [(-40.0, -68.0), (-40.0, -12.0), (-12.0, -40.0), (-68.0, -40.0)];
        for (ex, ez) in expected {
            let found = t.props.iter().any(|p| {
                matches!(p.kind, PropKind::WallEW | PropKind::WallNS)
                    && (p.offset.x - ex).abs() < 1e-3
                    && (p.offset.z - ez).abs() < 1e-3
            });
            assert!(
                found,
                "missing perimeter wall at ({ex}, {ez}); got: {:?}",
                t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn t_junction_perimeter_cell_emits_a_stub_to_the_divider() {
        // 3×5: single row of rooms with a vertical divider at col 2.
        // Cell (0, 2) is the top perimeter cell where the divider meets
        // the outer wall — it must emit a centred WallNS stub that
        // reaches down into the divider at (1, 2), otherwise there's a
        // visible ~56-unit gap between the perimeter WallEW and the
        // divider WallNS below.
        let json = synthetic_mapgen(&["wwwww", "w.w.w", "wwwww"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Cell (0, 2) centre: x = (2−2.5)×80 = −40, z = (0−1.5)×80 = −120.
        // The stub is a centred WallNS at (−40, −120).
        let has_stub = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-120.0)).abs() < 1e-3
        });
        assert!(
            has_stub,
            "top T-junction cell (0, 2) should emit a centred WallNS \
             stub to bridge the divider; got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
        // Same for the bottom T-junction (2, 2), row 2 centre z = +40.
        let has_stub_s = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - 40.0).abs() < 1e-3
        });
        assert!(has_stub_s, "bottom T-junction (2, 2) should also emit a stub");
    }

    #[test]
    fn a_cell_never_emits_more_than_one_wall_per_axis() {
        // Regression: divider + diagonal + perimeter rules were all
        // additive on the same axis, so a divider cell whose diagonals
        // were also interior emitted N-outer + S-outer + centred = three
        // parallel WallEW segments in the same 80-unit cell (visible
        // in-game as tripled walls / windows). Cross-shaped divider
        // pattern reproduces it: a wall cell with rooms N, S, E, W and
        // interior on every diagonal.
        let json = synthetic_mapgen(&[
            "wwwww",
            "w...w",
            "w.w.w",
            "w...w",
            "wwwww",
        ]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        use std::collections::HashMap;
        let mut per_cell_ew: HashMap<(i32, i32), usize> = HashMap::new();
        let mut per_cell_ns: HashMap<(i32, i32), usize> = HashMap::new();
        for p in &t.props {
            let cx = (p.offset.x / CDDA_TILE).round() as i32;
            let cz = (p.offset.z / CDDA_TILE).round() as i32;
            match p.kind {
                PropKind::WallEW | PropKind::WindowEW => {
                    *per_cell_ew.entry((cx, cz)).or_default() += 1;
                }
                PropKind::WallNS | PropKind::WindowNS => {
                    *per_cell_ns.entry((cx, cz)).or_default() += 1;
                }
                _ => {}
            }
        }
        for (cell, n) in &per_cell_ew {
            assert!(*n <= 1, "cell {cell:?}: {n} WallEW emissions on same axis");
        }
        for (cell, n) in &per_cell_ns {
            assert!(*n <= 1, "cell {cell:?}: {n} WallNS emissions on same axis");
        }
    }

    #[test]
    fn divider_between_two_rooms_extends_through_its_full_run() {
        // 4×5: two 1×1 rooms separated by a divider at col 2.
        // The two middle cells (1, 2) and (2, 2) are dividers (interior
        // E + interior W) and emit a centred WallNS each. The perimeter
        // cells (0, 2) and (3, 2) at the T-junctions emit a centred
        // WallNS STUB — different concern from divider emission, tested
        // by `t_junction_perimeter_cell_emits_a_stub_to_the_divider`.
        let json = synthetic_mapgen(&["wwwww", "w.w.w", "w.w.w", "wwwww"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Cx=2.5, Cz=2.0. Divider centre wall at x=−40; rows 1 and 2
        // at z=−80 and z=0.
        let mid_1 = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-80.0)).abs() < 1e-3
        });
        let mid_2 = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && p.offset.z.abs() < 1e-3
        });
        assert!(mid_1 && mid_2, "the divider emits at rows 1 and 2");
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
