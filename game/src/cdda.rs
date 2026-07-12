//! CDDA mapgen importer — parse a Cataclysm: Dark Days Ahead mapgen
//! JSON entry into a `Template` the stamp machinery can place.
//!
//! MVP scope: inline-palette buildings only (no external palettes),
//! single z-level. CDDA terrain/furniture ids map into the prop
//! vocabulary — walls/fences → Wall, chairs → Chair, counters/desks/
//! tables → Table — and everything else (floors, doors, windows,
//! outdoor ground, unmodelled furniture) is skipped. Enough to render
//! a recognisable building outline with furniture; the mapping grows
//! as the prop set does. The eventual optimisation is to bake the
//! template at build time rather than parse JSON at load.
//!
//! The garage layout under assets/cdda/ is CC-BY-SA 3.0 CDDA content —
//! see assets/cdda/ATTRIBUTION.md.

use std::collections::HashMap;
use std::fmt;

use bevy_ecs::prelude::*;
use bevy_math::Vec3;
use serde::Deserialize;
use serde_json::Value;

use crate::obs;
use crate::template::{stamp_template, Prop, PropKind, Template};

/// World units per CDDA tile. Wall props are sized to this (see the
/// Wall collider in template.rs and the Wall appearance in scene.rs,
/// both 80) so the grid tiles seamlessly.
pub const CDDA_TILE: f32 = 80.0;

/// Roof elevation — matches the wall height so the slab caps the walls.
pub const ROOF_HEIGHT: f32 = 220.0;

/// The embedded garage mapgen (CC-BY-SA 3.0, CleverRaven / CDDA).
const GARAGE_JSON: &str = include_str!("../assets/cdda/garage.json");

/// Where the imported garage lands — a landmark north-east of spawn,
/// clear of the central clearing.
pub const GARAGE_ANCHOR: Vec3 = Vec3::new(1600.0, 0.0, -1600.0);

/// Import failures — surfaced (sacred), never swallowed.
#[derive(Debug, PartialEq, Eq)]
pub enum CddaError {
    Parse(String),
    NotFound(String),
    NoObject(String),
    UnsupportedPalettes(String),
}

impl fmt::Display for CddaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CddaError::Parse(m) => write!(f, "CDDA mapgen parse error: {m}"),
            CddaError::NotFound(n) => write!(f, "CDDA mapgen '{n}' not found in file"),
            CddaError::NoObject(n) => write!(f, "CDDA mapgen '{n}' has no object"),
            CddaError::UnsupportedPalettes(n) => write!(
                f,
                "CDDA mapgen '{n}' uses external palettes — not supported (inline only)"
            ),
        }
    }
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    om_terrain: Value,
    object: Option<Obj>,
}

#[derive(Deserialize)]
struct Obj {
    #[serde(default)]
    rows: Vec<String>,
    #[serde(default)]
    terrain: HashMap<String, Value>,
    #[serde(default)]
    furniture: HashMap<String, Value>,
    #[serde(default)]
    palettes: Vec<Value>,
}

/// Does an om_terrain value (a string, or a nested array of strings)
/// name this om_terrain?
fn om_matches(v: &Value, name: &str) -> bool {
    match v {
        Value::String(s) => s == name,
        Value::Array(a) => a.iter().any(|x| om_matches(x, name)),
        _ => false,
    }
}

/// First id string in a CDDA terrain/furniture value, which may be a
/// bare string, an array of ids, or [id, weight] pairs.
fn first_id(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s.as_str()),
        Value::Array(a) => a.iter().find_map(first_id),
        _ => None,
    }
}

/// Map a cell's char to a prop, resolving furniture first (it sits on
/// the floor) then terrain. Unmapped cells → None (skipped).
fn cell_to_prop(
    ch: char,
    terrain: &HashMap<String, Value>,
    furniture: &HashMap<String, Value>,
) -> Option<PropKind> {
    let key = ch.to_string();
    if let Some(f) = furniture.get(&key).and_then(first_id) {
        if f.contains("chair") {
            return Some(PropKind::Chair);
        }
        if f.contains("counter") || f.contains("desk") || f.contains("table") {
            return Some(PropKind::Table);
        }
        // Known furniture we don't model yet → skip; don't fall to terrain.
        return None;
    }
    if let Some(t) = terrain.get(&key).and_then(first_id) {
        if (t.contains("wall") || t.contains("fence")) && !t.contains("gate") {
            return Some(PropKind::Wall);
        }
    }
    None
}

/// Parse a CDDA mapgen JSON file, select the entry named `om_terrain`,
/// and build a centred `Template` (col → +x, row → +z, grid centred on
/// the anchor).
pub fn mapgen_to_template(
    json: &str,
    om_terrain: &str,
    tile_size: f32,
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
    if !obj.palettes.is_empty() {
        return Err(CddaError::UnsupportedPalettes(om_terrain.to_string()));
    }

    let height = obj.rows.len();
    let width = obj.rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);

    // Pass 1: base prop kind per cell (walls are the plain Wall kind).
    let grid: Vec<Vec<Option<PropKind>>> = obj
        .rows
        .iter()
        .map(|row| {
            let mut cells: Vec<Option<PropKind>> = row
                .chars()
                .map(|ch| cell_to_prop(ch, &obj.terrain, &obj.furniture))
                .collect();
            cells.resize(width, None);
            cells
        })
        .collect();
    let is_wall = |r: isize, c: isize| -> bool {
        r >= 0
            && c >= 0
            && (r as usize) < height
            && (c as usize) < width
            && grid[r as usize][c as usize] == Some(PropKind::Wall)
    };

    // Pass 2: emit props, orienting each wall by its neighbours so a run
    // reads as a thin wall rather than a row of full-tile blocks.
    // Grid is centred on the anchor: col → +x, row → +z.
    let cx = width as f32 / 2.0;
    let cz = height as f32 / 2.0;
    let mut props = Vec::new();
    for r in 0..height {
        for c in 0..width {
            let Some(base) = grid[r][c] else { continue };
            let kind = if base == PropKind::Wall {
                let vertical =
                    is_wall(r as isize - 1, c as isize) || is_wall(r as isize + 1, c as isize);
                let horizontal =
                    is_wall(r as isize, c as isize - 1) || is_wall(r as isize, c as isize + 1);
                match (vertical, horizontal) {
                    (true, false) => PropKind::WallNS,
                    (false, true) => PropKind::WallEW,
                    _ => PropKind::Wall, // corner, junction, or isolated
                }
            } else {
                base
            };
            let x = (c as f32 - cx) * tile_size;
            let z = (r as f32 - cz) * tile_size;
            props.push(Prop { offset: Vec3::new(x, 0.0, z), kind });
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
                props.push(Prop {
                    offset: Vec3::new(x, height_y, z),
                    kind: PropKind::Roof,
                });
            }
        }
    }
    Ok(props)
}

/// The garage, imported from the embedded CDDA mapgen — ground floor
/// (walls + furniture) plus its roof z-level capping the building.
pub fn garage_template() -> Result<Template, CddaError> {
    let mut t = mapgen_to_template(GARAGE_JSON, "s_garage_1", CDDA_TILE)?;
    let roof = roof_to_props(GARAGE_JSON, "s_garage_roof_1", CDDA_TILE, ROOF_HEIGHT)?;
    t.props.extend(roof);
    Ok(t)
}

/// Startup system — stamp the imported garage. On import failure the
/// error surfaces on the obs bus and the world continues (the garage
/// just doesn't appear) rather than crashing.
pub fn setup_cdda_buildings(mut commands: Commands) {
    match garage_template() {
        Ok(t) => {
            stamp_template(&mut commands, &t, GARAGE_ANCHOR);
        }
        Err(e) => obs::emit(&format!("[cdda] garage import failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- helper-level tests (pass regardless of assembly stub) ---

    #[test]
    fn first_id_handles_string_and_weighted_array() {
        assert_eq!(first_id(&json!("t_wall_log")), Some("t_wall_log"));
        assert_eq!(
            first_id(&json!([["t_pavement", 10], "t_dirt"])),
            Some("t_pavement")
        );
        assert_eq!(first_id(&json!(5)), None);
    }

    #[test]
    fn om_matches_bare_and_nested() {
        assert!(om_matches(&json!("s_garage_1"), "s_garage_1"));
        assert!(om_matches(&json!(["s_garage_1"]), "s_garage_1"));
        assert!(!om_matches(&json!(["s_garage_2"]), "s_garage_1"));
    }

    #[test]
    fn cells_map_to_the_prop_vocabulary() {
        let terrain: HashMap<String, Value> = [
            ("w".to_string(), json!("t_wall_log")),
            ("W".to_string(), json!("t_chainfence")),
            ("^".to_string(), json!("t_chaingate_c")),
            (".".to_string(), json!("t_thconc_floor")),
        ]
        .into_iter()
        .collect();
        let furniture: HashMap<String, Value> = [
            ("h".to_string(), json!("f_chair")),
            ("c".to_string(), json!("f_counter")),
            ("t".to_string(), json!("f_toilet")),
        ]
        .into_iter()
        .collect();

        assert_eq!(cell_to_prop('w', &terrain, &furniture), Some(PropKind::Wall));
        assert_eq!(cell_to_prop('W', &terrain, &furniture), Some(PropKind::Wall));
        assert_eq!(cell_to_prop('^', &terrain, &furniture), None); // gate skipped
        assert_eq!(cell_to_prop('h', &terrain, &furniture), Some(PropKind::Chair));
        assert_eq!(cell_to_prop('c', &terrain, &furniture), Some(PropKind::Table));
        assert_eq!(cell_to_prop('t', &terrain, &furniture), None); // toilet unmodelled
        assert_eq!(cell_to_prop('.', &terrain, &furniture), None); // floor skipped
        assert_eq!(cell_to_prop(' ', &terrain, &furniture), None); // unknown
    }

    // --- assembly-level tests (RED against the stub) ---

    #[test]
    fn imports_garage_with_oriented_walls_furniture_and_roof() {
        let t = garage_template().expect("garage should import");
        let n = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
        let walls = n(PropKind::Wall) + n(PropKind::WallNS) + n(PropKind::WallEW);
        assert!(walls > 10, "expected a wall outline, got {walls}");
        assert!(
            n(PropKind::WallNS) + n(PropKind::WallEW) > 0,
            "walls should be oriented into thin runs"
        );
        assert!(
            n(PropKind::Chair) + n(PropKind::Table) > 0,
            "expected some furniture"
        );
        assert!(n(PropKind::Roof) > 0, "expected a roof");
        // The roof sits at ROOF_HEIGHT.
        assert!(
            t.props
                .iter()
                .any(|p| p.kind == PropKind::Roof && p.offset.y == ROOF_HEIGHT)
        );
        // Grid is centred on the anchor: offsets straddle zero on both axes.
        assert!(t.props.iter().any(|p| p.offset.x < 0.0));
        assert!(t.props.iter().any(|p| p.offset.x > 0.0));
    }

    #[test]
    fn unknown_om_terrain_is_a_surfaced_error() {
        let err = mapgen_to_template(GARAGE_JSON, "s_no_such_building", CDDA_TILE).unwrap_err();
        assert_eq!(err, CddaError::NotFound("s_no_such_building".to_string()));
    }

    #[test]
    fn palette_using_entry_is_rejected() {
        // s_garage (unlike s_garage_1) references parametrized_walls_palette.
        let err = mapgen_to_template(GARAGE_JSON, "s_garage", CDDA_TILE).unwrap_err();
        assert_eq!(err, CddaError::UnsupportedPalettes("s_garage".to_string()));
    }
}
