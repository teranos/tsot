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

use crate::chunk::{ChunkCoord, CHUNK_SIZE};
use crate::hash::wang_hash;
use crate::obs;
use crate::template::{Prop, PropKind, Template};

/// World units per CDDA tile. Wall props are sized to this (see the
/// Wall collider in template.rs and the Wall appearance in scene.rs,
/// both 80) so the grid tiles seamlessly.
pub const CDDA_TILE: f32 = 80.0;

/// Roof elevation — matches the wall height so the slab caps the walls.
pub const ROOF_HEIGHT: f32 = 220.0;

/// The embedded garage mapgen (CC-BY-SA 3.0, CleverRaven / CDDA).
const GARAGE_JSON: &str = include_str!("../assets/cdda/garage.json");

/// Buildings are rarer than campsites — roughly 1 chunk in 20.
const BUILDING_CHUNK_CHANCE: u32 = u32::MAX / 20;
const BUILDING_SALT: u32 = 0xB1D6_5175;
const BUILDING_PICK_SALT: u32 = 0xB1D6_9CE5;
/// Square half-extent around a building kept clear of trees, so the
/// forest doesn't grow through the walls — reads as a yard around it.
pub const BUILDING_FOOTPRINT_HALF: f32 = 1050.0;
/// Keep buildings well clear of the central clearing + trail; they're
/// big, so exclude their footprint reach plus a margin.
const BUILDING_CLEARING_EXCLUSION: f32 = 2000.0;
const BUILDING_TRAIL_HALF: f32 = 220.0 + BUILDING_FOOTPRINT_HALF;

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

/// Parsed building templates, cached once at startup so the chunk
/// streamer stamps from memory instead of re-parsing JSON per chunk.
#[derive(Resource, Default)]
pub struct BuildingTemplates(pub Vec<Template>);

/// Parse every building we ship, once. Import failures surface on the
/// obs bus (sacred); the building simply won't appear.
pub fn load_building_templates() -> BuildingTemplates {
    let mut templates = Vec::new();
    match garage_template() {
        Ok(t) => templates.push(t),
        Err(e) => obs::emit(&format!("[cdda] garage import failed: {e}")),
    }
    BuildingTemplates(templates)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn building_templates_load_the_garage() {
        let t = load_building_templates();
        assert!(!t.0.is_empty(), "garage should parse");
        assert!(t.0[0].props.len() > 50, "garage should be many props");
    }

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
