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
    // Resolve referenced palettes into flat char→id maps, then overlay
    // the inline terrain/furniture (inline overrides the palettes).
    let (mut terrain, mut furniture) = if obj.palettes.is_empty() {
        (HashMap::new(), HashMap::new())
    } else {
        crate::palette::resolve(&obj.palettes, seed)
    };
    for (sym, val) in &obj.terrain {
        if let (Some(ch), Some(id)) = (sym.chars().next(), first_id(val)) {
            terrain.insert(ch, id.to_string());
        }
    }
    for (sym, val) in &obj.furniture {
        if let (Some(ch), Some(id)) = (sym.chars().next(), first_id(val)) {
            furniture.insert(ch, id.to_string());
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
                .map(|ch| cell_to_prop(ch, &terrain, &furniture))
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

    // Interior side of the horizontal run containing (r, c): walk the
    // contiguous solid cells left and right, aggregate whether any of
    // them has interior directly N or S. Ditto for vertical.
    let horiz_run_interior = |r: i32, c: i32| -> (bool, bool) {
        let (mut int_n, mut int_s) = (false, false);
        let mut cc = c;
        while is_solid(r, cc) {
            if is_interior(r - 1, cc) {
                int_n = true;
            }
            if is_interior(r + 1, cc) {
                int_s = true;
            }
            cc -= 1;
        }
        let mut cc = c + 1;
        while is_solid(r, cc) {
            if is_interior(r - 1, cc) {
                int_n = true;
            }
            if is_interior(r + 1, cc) {
                int_s = true;
            }
            cc += 1;
        }
        (int_n, int_s)
    };
    let vert_run_interior = |r: i32, c: i32| -> (bool, bool) {
        let (mut int_e, mut int_w) = (false, false);
        let mut rr = r;
        while is_solid(rr, c) {
            if is_interior(rr, c + 1) {
                int_e = true;
            }
            if is_interior(rr, c - 1) {
                int_w = true;
            }
            rr -= 1;
        }
        let mut rr = r + 1;
        while is_solid(rr, c) {
            if is_interior(rr, c + 1) {
                int_e = true;
            }
            if is_interior(rr, c - 1) {
                int_w = true;
            }
            rr += 1;
        }
        (int_e, int_w)
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
    // rules; every other prop stays centred on its tile.
    let cx = width as f32 / 2.0;
    let cz = height as f32 / 2.0;
    let half = tile_size / 2.0;
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

            let horiz = is_solid(r, c - 1) || is_solid(r, c + 1);
            let vert = is_solid(r - 1, c) || is_solid(r + 1, c);
            if !horiz && !vert {
                // Isolated pillar: full-tile at centre.
                emit(cx_world, cz_world, base);
                continue;
            }

            let (hn, hs) = if horiz {
                horiz_run_interior(r, c)
            } else {
                (false, false)
            };
            let (ve, vw) = if vert {
                vert_run_interior(r, c)
            } else {
                (false, false)
            };
            if !hn && !hs && !ve && !vw {
                // Runs present but no interior anywhere in perp (e.g. a
                // + cross-junction fully surrounded by walls). Fall back
                // to a full-tile centred block.
                emit(cx_world, cz_world, base);
                continue;
            }

            // T-junction skip: at a cell where BOTH runs are active, a
            // one-sided end of a DIVIDER run would poke a wall pillar
            // into (or past) the perpendicular perimeter wall. Skip that
            // side's emission — the perimeter takes care of the visual
            // meeting point, and the next cell in the divider run picks
            // up the wall geometry. Doesn't fire at true corners (where
            // the run is single-side interior, not divider) or at
            // divider ends that aren't at a perpendicular perimeter.
            let horiz_one_sided = is_solid(r, c - 1) != is_solid(r, c + 1);
            let vert_one_sided = is_solid(r - 1, c) != is_solid(r + 1, c);
            let skip_horiz = horiz && hn && hs && horiz_one_sided && vert;
            let skip_vert = vert && ve && vw && vert_one_sided && horiz;

            if horiz && (hn || hs) && !skip_horiz {
                // (true, true) divider → +z convention.
                // (false, true) interior south → +z. (true, false) → -z.
                let z_off = match (hn, hs) {
                    (true, true) | (false, true) => half,
                    (true, false) => -half,
                    (false, false) => 0.0,
                };
                emit(cx_world, cz_world + z_off, ew_kind(base));
            }
            if vert && (ve || vw) && !skip_vert {
                // (true, true) divider → +x convention.
                // (true, false) interior east → +x. (false, true) → -x.
                let x_off = match (ve, vw) {
                    (true, true) | (true, false) => half,
                    (false, true) => -half,
                    (false, false) => 0.0,
                };
                emit(cx_world + x_off, cz_world, ns_kind(base));
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
    fn perimeter_wall_shifts_toward_interior() {
        // 3×3 room: solid perimeter, one floor at (1, 1). The top-middle
        // wall at (0, 1) has interior directly south — shift +tile/2 in z.
        let json = synthetic_mapgen(&["www", "w.w", "www"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Cell (0, 1) centre: x = (1 - 1.5) * 80 = -40, z = (0 - 1.5) * 80 = -120.
        // Shift +tile/2 (=40) in z toward interior south → z = -80.
        let has_shifted = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallEW)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-80.0)).abs() < 1e-3
        });
        assert!(
            has_shifted,
            "top-middle wall should shift toward interior south; got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn nw_corner_emits_two_segments_on_interior_edges() {
        let json = synthetic_mapgen(&["www", "w.w", "www"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        let has_horiz = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallEW)
                && (p.offset.x - (-120.0)).abs() < 1e-3
                && (p.offset.z - (-80.0)).abs() < 1e-3
        });
        let has_vert = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-80.0)).abs() < 1e-3
                && (p.offset.z - (-120.0)).abs() < 1e-3
        });
        assert!(has_horiz, "NW corner should emit a horizontal segment on its S edge");
        assert!(has_vert, "NW corner should emit a vertical segment on its E edge");
    }

    #[test]
    fn divider_between_two_rooms_shifts_positive() {
        // 3×5 mapgen: two 1×1 rooms separated by a divider at (1, 2).
        let json = synthetic_mapgen(&["wwwww", "w.w.w", "wwwww"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Cell (1, 2) centre: x = (2 - 2.5) * 80 = -40, z = (1 - 1.5) * 80 = -40.
        let has_divider = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - 0.0).abs() < 1e-3
                && (p.offset.z - (-40.0)).abs() < 1e-3
        });
        assert!(
            has_divider,
            "divider vertical segment should shift +tile/2 in x; got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn windows_use_the_same_placement_rules_as_walls() {
        let json = synthetic_mapgen(&["w:w", "w.w", "www"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        let has_window = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WindowEW)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-80.0)).abs() < 1e-3
        });
        assert!(
            has_window,
            "window should follow WallEW placement (shift +tile/2 in z); got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn t_junction_divider_doesnt_protrude_into_the_perimeter_cell() {
        // 4×5: two 1×1 rooms separated by a divider at col 2. Perimeter
        // walls all around. The T-junction cells (0, 2) and (3, 2) are
        // where the divider meets the top and bottom perimeter — the
        // vertical divider should NOT emit at those cells (the perimeter
        // wall + the next cell's vertical form the T cleanly).
        let json = synthetic_mapgen(&["wwwww", "w.w.w", "w.w.w", "wwwww"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // For 4×5 with tile=80: cx=2.5, cz=2.0.
        // Cell (0, 2) centre: x=-40, z=-160. Vert emission would sit at
        // (col+40, row_center) = (0, -160). Assert none there.
        let has_top_protrusion = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && p.offset.x.abs() < 1e-3
                && (p.offset.z - (-160.0)).abs() < 1e-3
        });
        assert!(
            !has_top_protrusion,
            "top T-junction should not emit a vertical divider at (0, 2); got: {:?}",
            t.props.iter().map(|p| (p.kind, p.offset)).collect::<Vec<_>>()
        );
        // Same for the bottom T-junction (row 3).
        let has_bottom_protrusion = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && p.offset.x.abs() < 1e-3
                && (p.offset.z - 80.0).abs() < 1e-3
        });
        assert!(!has_bottom_protrusion, "bottom T-junction should not emit vertical");
        // But the divider IS emitted at the middle cells (1, 2) and (2, 2).
        let mid_1 = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && p.offset.x.abs() < 1e-3
                && (p.offset.z - (-80.0)).abs() < 1e-3
        });
        let mid_2 = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && p.offset.x.abs() < 1e-3
                && p.offset.z.abs() < 1e-3
        });
        assert!(mid_1 && mid_2, "the divider still emits at rows 1 and 2");
    }

    #[test]
    fn nw_corner_still_emits_two_segments() {
        // Guard rail: the T-junction skip must NOT affect true corners
        // (where the perp side is exterior, not interior). Same 3×3 room
        // as the perimeter/corner tests.
        let json = synthetic_mapgen(&["www", "w.w", "www"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        let has_vert = t.props.iter().any(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-80.0)).abs() < 1e-3
                && (p.offset.z - (-120.0)).abs() < 1e-3
        });
        assert!(has_vert, "NW corner vertical must still emit");
    }

    #[test]
    fn unknown_om_terrain_is_a_surfaced_error() {
        let json = synthetic_mapgen(&["www", "w.w", "www"]);
        let err = mapgen_to_template(&json, "s_no_such_building", CDDA_TILE, 0).unwrap_err();
        assert_eq!(err, CddaError::NotFound("s_no_such_building".to_string()));
    }
}
