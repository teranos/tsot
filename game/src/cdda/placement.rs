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
    // the inline terrain / furniture (inline overrides the palettes).
    // Almost all furniture is dropped by `cell_to_prop`; toilets are
    // the current carve-out.
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

    // Pass 2: RUN-BASED.
    //
    // The previous per-cell approach emitted one prop per wall cell,
    // producing a mosaic of 80-wide bars that had to hope they lined up
    // at seams — every visual artifact was a seam where two per-cell
    // decisions met. Now:
    //
    //   1. For each wall/window cell, classify which SLOTS it emits into
    //      (N/S/E/W outer edges + centred EW/NS divider + stubs). This
    //      is the same body of rules as before — perimeter, divider,
    //      diagonal-corner, run-scope alignment, T-junction stub.
    //   2. Collect segments as (slot_kind, line, idx, base, colour) tuples
    //      keyed to a lattice: horizontal slots by row (line=row) and
    //      column (idx=col along the run); vertical slots by col and row.
    //   3. Sort + walk the segments, coalescing contiguous same-key
    //      same-idx+1 segments into a single RUN.
    //   4. Emit one prop per run, sized to cover the whole run in world
    //      units. Adjacent perimeter cells become ONE long prop — no
    //      seams between pieces because they ARE one piece.
    //
    // Non-wall props (fence, other) still emit one-per-cell.
    let cx = width as f32 / 2.0;
    let cz = height as f32 / 2.0;
    let half = tile_size / 2.0;
    const WALL_HALF_THICKNESS: f32 = 12.0;
    const WALL_THICKNESS: f32 = 2.0 * WALL_HALF_THICKNESS;
    let mut props = Vec::new();

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum SlotKind {
        NorthOuter, // horizontal, N outer edge of row
        SouthOuter, // horizontal, S outer edge of row
        CentreEW,   // horizontal, cell-centre of row (divider or stub)
        EastOuter,  // vertical, E outer edge of col
        WestOuter,  // vertical, W outer edge of col
        CentreNS,   // vertical, cell-centre of col (divider or stub)
    }
    #[derive(Clone, Copy)]
    struct Seg {
        slot_kind: SlotKind,
        line: i32, // horizontal slots: row; vertical slots: col
        idx: i32,  // horizontal slots: col; vertical slots: row (position along run)
        base: PropKind,
        color: Option<[f32; 3]>,
    }
    let mut segments: Vec<Seg> = Vec::new();

    for r_idx in 0..height {
        for c_idx in 0..width {
            let Some((base, color)) = grid[r_idx][c_idx] else { continue };
            let r = r_idx as i32;
            let c = c_idx as i32;
            let cx_world = (c_idx as f32 - cx) * tile_size;
            let cz_world = (r_idx as f32 - cz) * tile_size;

            // Fence: one prop per cell, oriented by fence neighbours.
            // Kept per-cell for now — fence visual is post+rail per cell,
            // not a single long bar.
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
                    _ => PropKind::Fence,
                };
                let offset = Vec3::new(cx_world, 0.0, cz_world);
                props.push(match color {
                    Some(col) => Prop::colored(offset, kind, col),
                    None => Prop::at(offset, kind),
                });
                continue;
            }

            let is_wall_or_window = matches!(base, PropKind::Wall | PropKind::Window);
            if !is_wall_or_window {
                // Chair / Table / other centred prop.
                let offset = Vec3::new(cx_world, 0.0, cz_world);
                props.push(match color {
                    Some(col) => Prop::colored(offset, base, col),
                    None => Prop::at(offset, base),
                });
                continue;
            }

            // Wall / window classification: which slots does this cell
            // contribute a segment to? Rules unchanged from per-cell
            // version; only the EMIT step differs (segments now, coalesced).
            let (hn, hs) = horiz_at(r, c);
            let (ve, vw) = vert_at(r, c);
            let is_ext = |rr: i32, cc: i32| -> bool {
                if rr < 0 || cc < 0 || (rr as usize) >= height || (cc as usize) >= width {
                    return true; // OOB is exterior for perimeter emission
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

            let divider_ew = hn && hs;
            let divider_ns = ve && vw;

            let d_nw = is_interior(r - 1, c - 1);
            let d_ne = is_interior(r - 1, c + 1);
            let d_sw = is_interior(r + 1, c - 1);
            let d_se = is_interior(r + 1, c + 1);
            let n_open = !is_solid(r - 1, c);
            let s_open = !is_solid(r + 1, c);
            let e_open = !is_solid(r, c + 1);
            let w_open = !is_solid(r, c - 1);
            if d_se { emit_n |= n_open; emit_w |= w_open; }
            if d_sw { emit_n |= n_open; emit_e |= e_open; }
            if d_ne { emit_s |= s_open; emit_w |= w_open; }
            if d_nw { emit_s |= s_open; emit_e |= e_open; }

            if divider_ew { emit_n = false; emit_s = false; }
            if divider_ns { emit_e = false; emit_w = false; }

            if !emit_n && !emit_s && !divider_ew
                && (is_solid(r, c - 1) || is_solid(r, c + 1))
            {
                let (run_n, run_s) = horiz_run_side(r, c);
                emit_n = run_n;
                emit_s = run_s;
            }
            if !emit_e && !emit_w && !divider_ns
                && (is_solid(r - 1, c) || is_solid(r + 1, c))
            {
                let (run_e, run_w) = vert_run_side(r, c);
                emit_e = run_e;
                emit_w = run_w;
            }

            let neighbour_is_ns_divider = |rr: i32, cc: i32| -> bool {
                is_solid(rr, cc) && is_interior(rr, cc + 1) && is_interior(rr, cc - 1)
            };
            let neighbour_is_ew_divider = |rr: i32, cc: i32| -> bool {
                is_solid(rr, cc) && is_interior(rr - 1, cc) && is_interior(rr + 1, cc)
            };
            let need_ns_stub = !divider_ns
                && (neighbour_is_ns_divider(r - 1, c) || neighbour_is_ns_divider(r + 1, c));
            let need_ew_stub = !divider_ew
                && (neighbour_is_ew_divider(r, c - 1) || neighbour_is_ew_divider(r, c + 1));

            // Add segments (horizontal slots use line=row + idx=col;
            // vertical slots use line=col + idx=row — the "line" is the
            // constant across a run, "idx" is the position along it).
            let h_seg = |sk: SlotKind| Seg { slot_kind: sk, line: r, idx: c, base, color };
            let v_seg = |sk: SlotKind| Seg { slot_kind: sk, line: c, idx: r, base, color };
            if emit_n { segments.push(h_seg(SlotKind::NorthOuter)); }
            if emit_s { segments.push(h_seg(SlotKind::SouthOuter)); }
            if divider_ew { segments.push(h_seg(SlotKind::CentreEW)); }
            if need_ew_stub { segments.push(h_seg(SlotKind::CentreEW)); }
            if emit_e { segments.push(v_seg(SlotKind::EastOuter)); }
            if emit_w { segments.push(v_seg(SlotKind::WestOuter)); }
            if divider_ns { segments.push(v_seg(SlotKind::CentreNS)); }
            if need_ns_stub { segments.push(v_seg(SlotKind::CentreNS)); }

            // Isolated fallback: wall cell with nothing else to emit
            // becomes a single centred segment (length 1 run).
            let anything = emit_n || emit_s || emit_e || emit_w
                || divider_ew || divider_ns || need_ns_stub || need_ew_stub;
            if !anything {
                segments.push(h_seg(SlotKind::CentreEW));
            }
        }
    }

    // Coalesce segments into runs.
    let slot_ord = |k: SlotKind| match k {
        SlotKind::NorthOuter => 0u8,
        SlotKind::SouthOuter => 1,
        SlotKind::CentreEW => 2,
        SlotKind::EastOuter => 3,
        SlotKind::WestOuter => 4,
        SlotKind::CentreNS => 5,
    };
    let base_ord = |b: PropKind| match b {
        PropKind::Wall => 0u8,
        PropKind::Window => 1,
        _ => 255,
    };
    let color_bits = |c: Option<[f32; 3]>| c.map(|a| [a[0].to_bits(), a[1].to_bits(), a[2].to_bits()]);
    segments.sort_by(|a, b| {
        (slot_ord(a.slot_kind), a.line, base_ord(a.base), color_bits(a.color), a.idx).cmp(
            &(slot_ord(b.slot_kind), b.line, base_ord(b.base), color_bits(b.color), b.idx),
        )
    });
    // Deduplicate identical segments (two rules can independently
    // request the same centred stub for the same cell).
    segments.dedup_by(|a, b| {
        a.slot_kind == b.slot_kind
            && a.line == b.line
            && a.base == b.base
            && a.color == b.color
            && a.idx == b.idx
    });

    // Junction cleanup: build a per-cell map of which slot kinds are
    // present, so runs can be SHORTENED at their ends when they meet a
    // perpendicular segment. Rule: NS-oriented runs (EastOuter,
    // WestOuter, CentreNS) shrink by WALL_THICKNESS at each end where
    // the end cell also carries an EW segment (NorthOuter, SouthOuter,
    // CentreEW). EW runs stay full-length. This eliminates the 24×24
    // corner overlap that produced z-fighting spikes in iso view: at a
    // corner or T-junction, the NS wall now stops flush against the EW
    // wall's inner face, no shared 3D volume.
    use std::collections::HashSet;
    let mut cell_ew_slots: HashSet<(i32, i32)> = HashSet::new();
    let mut cell_ns_slots: HashSet<(i32, i32)> = HashSet::new();
    for seg in &segments {
        let cell = match seg.slot_kind {
            SlotKind::NorthOuter | SlotKind::SouthOuter | SlotKind::CentreEW => {
                cell_ew_slots.insert((seg.line, seg.idx));
                (seg.line, seg.idx)
            }
            SlotKind::EastOuter | SlotKind::WestOuter | SlotKind::CentreNS => {
                cell_ns_slots.insert((seg.idx, seg.line));
                (seg.idx, seg.line)
            }
        };
        let _ = cell;
    }
    // Wall-material colour lookup — for a window run to render its
    // frame (bottom + top layers) in the neighbouring wall's material
    // colour, not the glass tint. Look at the cells one step off the
    // run's line (perpendicular direction) and take the first Wall
    // colour we find in the grid.
    let wall_color_near = |seg: &Seg| -> Option<[f32; 3]> {
        // The run lives on `seg.line` (row for horizontal, col for
        // vertical). Adjacent perpendicular cells are one step off in
        // the line direction.
        let (r0, c0, r1, c1) = match seg.slot_kind {
            SlotKind::NorthOuter | SlotKind::SouthOuter | SlotKind::CentreEW => {
                (seg.line - 1, seg.idx, seg.line + 1, seg.idx)
            }
            SlotKind::EastOuter | SlotKind::WestOuter | SlotKind::CentreNS => {
                (seg.idx, seg.line - 1, seg.idx, seg.line + 1)
            }
        };
        let lookup = |r: i32, c: i32| -> Option<[f32; 3]> {
            if r < 0 || c < 0 || (r as usize) >= height || (c as usize) >= width {
                return None;
            }
            match grid[r as usize][c as usize] {
                Some((PropKind::Wall, Some(col))) => Some(col),
                _ => None,
            }
        };
        lookup(r0, c0).or_else(|| lookup(r1, c1))
    };

    let mut i = 0;
    while i < segments.len() {
        let start = i;
        while i + 1 < segments.len()
            && segments[i + 1].slot_kind == segments[start].slot_kind
            && segments[i + 1].line == segments[start].line
            && segments[i + 1].base == segments[start].base
            && segments[i + 1].color == segments[start].color
            && segments[i + 1].idx == segments[i].idx + 1
        {
            i += 1;
        }
        let seg = segments[start];
        let end_idx = segments[i].idx;
        let run_cells = (end_idx - seg.idx + 1) as f32;
        let mut along_size = run_cells * tile_size;
        let idx_centre = (seg.idx as f32 + (run_cells - 1.0) * 0.5) * tile_size;

        // Junction shortening for NS runs.
        let mut idx_shift = 0.0_f32;
        let is_ns_run = matches!(
            seg.slot_kind,
            SlotKind::EastOuter | SlotKind::WestOuter | SlotKind::CentreNS
        );
        if is_ns_run {
            // Start cell (top of run) — check if it has an EW segment.
            let start_cell = (seg.idx, seg.line);
            if cell_ew_slots.contains(&start_cell) {
                along_size -= WALL_THICKNESS;
                idx_shift += WALL_HALF_THICKNESS;
            }
            // End cell (bottom of run).
            let end_cell = (end_idx, seg.line);
            if cell_ew_slots.contains(&end_cell) {
                along_size -= WALL_THICKNESS;
                idx_shift -= WALL_HALF_THICKNESS;
            }
        }
        // Guard against negative sizes (e.g., 1-cell NS run whose only
        // cell has EW segments would shorten to negative). Snap to a
        // tiny positive so the prop still renders somewhere sensible.
        if along_size < 1.0 {
            along_size = 1.0;
        }

        // Emit position/size/kind per slot.
        let (offset, size, kind) = match seg.slot_kind {
            SlotKind::NorthOuter => {
                let z = (seg.line as f32 - cz) * tile_size - half + WALL_HALF_THICKNESS;
                let x = idx_centre - cx * tile_size + idx_shift;
                (
                    Vec3::new(x, 0.0, z),
                    Vec3::new(along_size, 220.0, WALL_THICKNESS),
                    ew_kind(seg.base),
                )
            }
            SlotKind::SouthOuter => {
                let z = (seg.line as f32 - cz) * tile_size + half - WALL_HALF_THICKNESS;
                let x = idx_centre - cx * tile_size + idx_shift;
                (
                    Vec3::new(x, 0.0, z),
                    Vec3::new(along_size, 220.0, WALL_THICKNESS),
                    ew_kind(seg.base),
                )
            }
            SlotKind::CentreEW => {
                let z = (seg.line as f32 - cz) * tile_size;
                let x = idx_centre - cx * tile_size + idx_shift;
                (
                    Vec3::new(x, 0.0, z),
                    Vec3::new(along_size, 220.0, WALL_THICKNESS),
                    ew_kind(seg.base),
                )
            }
            SlotKind::EastOuter => {
                let x = (seg.line as f32 - cx) * tile_size + half - WALL_HALF_THICKNESS;
                let z = idx_centre - cz * tile_size + idx_shift;
                (
                    Vec3::new(x, 0.0, z),
                    Vec3::new(WALL_THICKNESS, 220.0, along_size),
                    ns_kind(seg.base),
                )
            }
            SlotKind::WestOuter => {
                let x = (seg.line as f32 - cx) * tile_size - half + WALL_HALF_THICKNESS;
                let z = idx_centre - cz * tile_size + idx_shift;
                (
                    Vec3::new(x, 0.0, z),
                    Vec3::new(WALL_THICKNESS, 220.0, along_size),
                    ns_kind(seg.base),
                )
            }
            SlotKind::CentreNS => {
                let x = (seg.line as f32 - cx) * tile_size;
                let z = idx_centre - cz * tile_size + idx_shift;
                (
                    Vec3::new(x, 0.0, z),
                    Vec3::new(WALL_THICKNESS, 220.0, along_size),
                    ns_kind(seg.base),
                )
            }
        };

        // Layered emission for window runs: instead of one full-height
        // glass panel (which reads as a translucent wall floor-to-
        // ceiling and doesn't look like a window), emit three stacked
        // props — bottom wall + glass strip + top wall — using the
        // neighbouring wall's material colour for the frame layers.
        if matches!(seg.base, PropKind::Window) {
            const SILL_TOP: f32 = 60.0;
            const HEAD_BOTTOM: f32 = 180.0;
            const HEIGHT: f32 = 220.0;
            let frame_color = wall_color_near(&seg);
            let bottom_size = Vec3::new(
                size.x,
                SILL_TOP,
                size.z,
            );
            let bottom_offset = Vec3::new(offset.x, 0.0, offset.z);
            let middle_size = Vec3::new(size.x, HEAD_BOTTOM - SILL_TOP, size.z);
            let middle_offset = Vec3::new(offset.x, SILL_TOP, offset.z);
            let top_size = Vec3::new(size.x, HEIGHT - HEAD_BOTTOM, size.z);
            let top_offset = Vec3::new(offset.x, HEAD_BOTTOM, offset.z);
            let wall_kind = ew_kind(PropKind::Wall);
            // Bottom + top use the wall variant (opaque, wall-coloured).
            let wall_variant = match kind {
                PropKind::WindowEW => PropKind::WallEW,
                PropKind::WindowNS => PropKind::WallNS,
                _ => wall_kind,
            };
            props.push(Prop::sized(bottom_offset, wall_variant, frame_color, bottom_size));
            props.push(Prop::sized(middle_offset, kind, seg.color, middle_size));
            props.push(Prop::sized(top_offset, wall_variant, frame_color, top_size));
        } else {
            props.push(Prop::sized(offset, kind, seg.color, size));
        }
        i += 1;
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
    fn perimeter_is_four_runs_one_per_side() {
        // Run-based: a 3×3 room's outer outline is FOUR long walls —
        // one per side — each spanning all 3 cells. Adjacent per-cell
        // segments coalesce into a single prop, so there are no seams
        // between them and the corners share the endpoint cells cleanly.
        let json = synthetic_mapgen(&["www", "w.w", "www"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Room centre at origin (3×3, cx=cz=1.5). Each side's run spans
        // 3 cells = 240 wide, sits at the outer edge (z or x = ±148),
        // centred on the run's axis (x or z = 0).
        // 3×3, cx=cz=1.5. Cell centres at −120, −40, 40. EW runs of 3
        // cells centred at −40 span the FULL width (240) since EW
        // extends to the corners. NS runs shorten by WALL_THICKNESS at
        // each end where the corner cell also carries an EW segment
        // (the NW/NE/SW/SE corners do), giving span 240 − 48 = 192.
        // This eliminates the 24×24 corner overlap that used to
        // z-fight and read as a distinct pillar in iso view.
        let sides = [
            (-40.0, -148.0, PropKind::WallEW, 240.0), // north
            (-40.0, 68.0, PropKind::WallEW, 240.0),   // south
            (68.0, -40.0, PropKind::WallNS, 192.0),   // east
            (-148.0, -40.0, PropKind::WallNS, 192.0), // west
        ];
        for (ex, ez, kind, expected_span) in sides {
            let found = t.props.iter().any(|p| {
                p.kind == kind
                    && (p.offset.x - ex).abs() < 1e-3
                    && (p.offset.z - ez).abs() < 1e-3
                    && p.size.map(|s| {
                        (matches!(kind, PropKind::WallEW) && (s.x - expected_span).abs() < 1e-3)
                            || (matches!(kind, PropKind::WallNS) && (s.z - expected_span).abs() < 1e-3)
                    }).unwrap_or(false)
            });
            assert!(
                found,
                "missing {kind:?} run at ({ex}, {ez}) with span {expected_span}; got: {:?}",
                t.props.iter()
                    .filter(|p| matches!(p.kind, PropKind::WallEW | PropKind::WallNS))
                    .map(|p| (p.kind, p.offset, p.size))
                    .collect::<Vec<_>>()
            );
        }
        // Exactly 4 wall props total (no extras).
        let count = t
            .props
            .iter()
            .filter(|p| matches!(p.kind, PropKind::WallEW | PropKind::WallNS))
            .count();
        assert_eq!(count, 4, "3×3 room should be 4 outer-side runs, got {count}");
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
    fn t_junction_perimeter_cell_stub_coalesces_with_divider() {
        // 3×5: rooms either side of a divider at col 2. Divider at
        // (1, 2) plus stubs at the T-junction perimeter cells (0, 2)
        // and (2, 2) all live on the col-2 CentreNS lattice, contiguous
        // rows 0..2 → coalesce into ONE run that reaches from the top
        // perimeter to the bottom perimeter (no gap possible because
        // it's a single prop).
        let json = synthetic_mapgen(&["wwwww", "w.w.w", "wwwww"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Col 2 centre x = −40. Run centre z = midpoint of row 0 and
        // row 2 = midpoint of (−120, 40) = −40. Span = 3 × 80 = 240.
        let full = t.props.iter().find(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-40.0)).abs() < 1e-3
        });
        assert!(
            full.is_some(),
            "stub + divider + stub should coalesce into one WallNS at (−40, −40); got: {:?}",
            t.props.iter()
                .filter(|p| matches!(p.kind, PropKind::WallNS))
                .map(|p| (p.offset, p.size))
                .collect::<Vec<_>>()
        );
        let sz = full.unwrap().size.unwrap();
        // 3-row span = 240 units, shortened by 24 at each junction end
        // (top and bottom perimeter cells carry EW segments) → 192.
        assert!(
            (sz.z - 192.0).abs() < 1e-3,
            "3-row run minus 48 units junction shortening = 192; got z={}",
            sz.z
        );
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
    fn divider_between_two_rooms_coalesces_into_one_run() {
        // 4×5: two 1×1 rooms separated by a divider at col 2. Divider
        // cells (1, 2) and (2, 2) both emit a CentreNS segment. The
        // T-junction perimeter cells (0, 2) and (3, 2) also emit
        // CentreNS stubs on the same col-2 lattice. All four segments
        // are contiguous rows 0..3 on col 2 → coalesce into ONE run
        // spanning the full 4 rows.
        let json = synthetic_mapgen(&["wwwww", "w.w.w", "w.w.w", "wwwww"]);
        let t = mapgen_to_template(&json, "tt", CDDA_TILE, 0).unwrap();
        // Col 2 centre x = (2−2.5)*80 = −40. Full 4-row run centre z =
        // midpoint of row 0 (z=−160) and row 3 (z=80) centres = −40.
        // Span = 4 * 80 = 320.
        let full_run = t.props.iter().find(|p| {
            matches!(p.kind, PropKind::WallNS)
                && (p.offset.x - (-40.0)).abs() < 1e-3
                && (p.offset.z - (-40.0)).abs() < 1e-3
        });
        assert!(
            full_run.is_some(),
            "divider + stubs should coalesce into one WallNS at (−40, −20); got: {:?}",
            t.props
                .iter()
                .filter(|p| matches!(p.kind, PropKind::WallNS))
                .map(|p| (p.offset, p.size))
                .collect::<Vec<_>>()
        );
        let sz = full_run.unwrap().size.expect("run-based prop carries size");
        // Full 4-row span = 320 units, minus WALL_THICKNESS (24) at
        // each end where the T-junction perimeter carries an EW
        // segment → 320 − 48 = 272. The NS wall stops flush against
        // the EW perimeter's inner face at top and bottom, no shared
        // 3D volume, no z-fight.
        assert!(
            (sz.z - 272.0).abs() < 1e-3,
            "coalesced run spans 4 rows minus junction shortening (272 units); got z={}",
            sz.z
        );
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
