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

// CDDA mapgen embedded from the build-time corpus (build.rs copies it
// out of the release pinned in CDDA_RELEASE — never vendored in git).
// CC-BY-SA 3.0, CleverRaven / CDDA; see assets/cdda/ATTRIBUTION.md.
const GARAGE_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/garage.json"));
/// The house mapgen — palette-driven (CC-BY-SA 3.0, CDDA).
const HOUSE_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/house01.json"));
/// A shed — CDDA has no standalone one, so this is an original inline
/// mapgen in the same format (ours, so it stays vendored in-tree).
const SHED_JSON: &str = include_str!("../assets/buildings/shed.json");

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

/// Map a furniture id to a prop. Seats → Chair, work surfaces → Table,
/// other solid furniture → a generic Furniture box; decorative bits
/// (plants, lamps, mailboxes…) → None (skipped).
fn furniture_prop(id: &str) -> Option<PropKind> {
    let has = |needles: &[&str]| needles.iter().any(|n| id.contains(n));
    if has(&["chair", "stool", "bench", "sofa", "armchair"]) {
        return Some(PropKind::Chair);
    }
    if has(&["table", "counter", "desk", "workbench"]) {
        return Some(PropKind::Table);
    }
    const SOLID: &[&str] = &[
        "bed", "dresser", "fridge", "oven", "stove", "sink", "toilet", "bookcase", "wardrobe",
        "cabinet", "locker", "rack", "shelf", "cupboard", "washer", "dryer", "dishwasher",
        "bathtub", "shower", "chest", "safe", "fireplace", "furnace", "piano", "crate",
        "entertainment", "displaycase", "glass_",
    ];
    if has(SOLID) {
        return Some(PropKind::Furniture);
    }
    None
}

/// Glass windows — a light-blue thin panel sitting in the wall line.
const WINDOW_COLOR: [f32; 3] = [0.50, 0.68, 0.82];

/// Wall/fence colour by material, so parametrized wall variation shows
/// as differently-coloured houses (brick/wood/concrete/log/…).
fn wall_color(id: &str) -> [f32; 3] {
    if id.contains("brick") {
        [0.55, 0.32, 0.27]
    } else if id.contains("concrete") || id.contains("thconc") || id.contains("cinder") {
        [0.56, 0.56, 0.60]
    } else if id.contains("metal") || id.contains("chain") {
        [0.46, 0.49, 0.53]
    } else if id.contains("log") {
        [0.40, 0.29, 0.17]
    } else if id.contains("glass") {
        [0.40, 0.55, 0.60]
    } else if id.contains("wood") || id.contains("wall_w") || id.contains("fence") {
        [0.52, 0.40, 0.25]
    } else {
        [0.48, 0.47, 0.50] // generic
    }
}

/// Map a cell's char to (prop kind, optional colour) via the resolved
/// char→id maps — furniture first (it sits on the floor), then terrain.
/// Walls carry a material colour. Unmapped → None.
fn cell_to_prop(
    ch: char,
    terrain: &HashMap<char, String>,
    furniture: &HashMap<char, String>,
) -> Option<(PropKind, Option<[f32; 3]>)> {
    if let Some(f) = furniture.get(&ch) {
        // Furniture char — its prop (or None); don't fall through to terrain.
        return furniture_prop(f).map(|k| (k, None));
    }
    if let Some(t) = terrain.get(&ch) {
        // A window is a translucent glass panel that sits in (and
        // orients with) the wall run — see-through from outside, drawn
        // in its own alpha pass. Kept as the base Window kind here;
        // pass 2 orients it NS/EW to match its wall run.
        if t.contains("window") {
            return Some((PropKind::Window, Some(WINDOW_COLOR)));
        }
        if (t.contains("wall") || t.contains("fence")) && !t.contains("gate") {
            return Some((PropKind::Wall, Some(wall_color(t))));
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
    let grid: Vec<Vec<Option<(PropKind, Option<[f32; 3]>)>>> = obj
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
    // A window continues a wall run, so it counts as wall-like both for
    // orienting its neighbours and for orienting itself.
    let is_wall_like = |r: isize, c: isize| -> bool {
        r >= 0
            && c >= 0
            && (r as usize) < height
            && (c as usize) < width
            && matches!(
                grid[r as usize][c as usize],
                Some((PropKind::Wall, _)) | Some((PropKind::Window, _))
            )
    };

    // Pass 2: emit props, orienting each wall by its neighbours so a run
    // reads as a thin wall rather than a row of full-tile blocks.
    // Grid is centred on the anchor: col → +x, row → +z.
    let cx = width as f32 / 2.0;
    let cz = height as f32 / 2.0;
    let mut props = Vec::new();
    for r in 0..height {
        for c in 0..width {
            let Some((base, color)) = grid[r][c] else { continue };
            let kind = if base == PropKind::Wall || base == PropKind::Window {
                let vertical = is_wall_like(r as isize - 1, c as isize)
                    || is_wall_like(r as isize + 1, c as isize);
                let horizontal = is_wall_like(r as isize, c as isize - 1)
                    || is_wall_like(r as isize, c as isize + 1);
                // Orient walls and windows the same way: thin across the
                // run's length, full-tile at corners/junctions.
                match (base, vertical, horizontal) {
                    (PropKind::Wall, true, false) => PropKind::WallNS,
                    (PropKind::Wall, false, true) => PropKind::WallEW,
                    (PropKind::Wall, _, _) => PropKind::Wall,
                    (PropKind::Window, true, false) => PropKind::WindowNS,
                    (PropKind::Window, false, true) => PropKind::WindowEW,
                    (PropKind::Window, _, _) => PropKind::Window,
                    _ => base,
                }
            } else {
                base
            };
            let offset = Vec3::new((c as f32 - cx) * tile_size, 0.0, (r as f32 - cz) * tile_size);
            props.push(match color {
                Some(col) => Prop::colored(offset, kind, col),
                None => Prop::at(offset, kind),
            });
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

/// The garage, imported from the embedded CDDA mapgen — ground floor
/// (walls + furniture) plus its roof z-level capping the building.
/// How many hash-varied house variants to pre-build. Each picks its
/// own variant palette (standard / abandoned / hoarder / survivor) +
/// fence/wall/lino, so building_index lands on visibly different houses.
pub const HOUSE_VARIANTS: u32 = 6;

pub fn garage_template() -> Result<Template, CddaError> {
    assemble_building(GARAGE_JSON, "s_garage_1", "s_garage_roof_1", 0)
}

/// The house at the canonical seed 0 (used by tests). The world uses
/// the seeded variants built in `load_building_templates`.
pub fn house_template() -> Result<Template, CddaError> {
    assemble_building(HOUSE_JSON, "house_01", "house_01_roof", 0)
}

/// A small shed (original inline mapgen, no palettes).
pub fn shed_template() -> Result<Template, CddaError> {
    assemble_building(SHED_JSON, "shed_1", "shed_roof", 0)
}

/// Ground floor (walls + furniture, palettes resolved at `seed`) + roof.
fn assemble_building(
    json: &str,
    ground_om: &str,
    roof_om: &str,
    seed: u32,
) -> Result<Template, CddaError> {
    let mut t = mapgen_to_template(json, ground_om, CDDA_TILE, seed)?;
    let roof = roof_to_props(json, roof_om, CDDA_TILE, ROOF_HEIGHT)?;
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
    let mut specs: Vec<(&str, Result<Template, CddaError>)> =
        vec![("garage", garage_template()), ("shed", shed_template())];
    for seed in 0..HOUSE_VARIANTS {
        specs.push((
            "house",
            assemble_building(HOUSE_JSON, "house_01", "house_01_roof", seed),
        ));
    }
    let mut templates = Vec::new();
    for (name, result) in specs {
        match result {
            Ok(t) => templates.push(t),
            Err(e) => obs::emit(&format!("[cdda] {name} import failed: {e}")),
        }
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

/// Deterministic quarter-turn rotation (0..4) for a building-chunk, so
/// two buildings of the same type face different ways.
pub fn building_rotation(c: ChunkCoord) -> u8 {
    (wang_hash(c.x, c.z, BUILDING_ROT_SALT) % 4) as u8
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
        let s = |v: &str| v.to_string();
        let terrain: HashMap<char, String> = [
            ('w', s("t_wall_log")),
            ('W', s("t_chainfence")),
            ('^', s("t_chaingate_c")),
            ('.', s("t_thconc_floor")),
        ]
        .into_iter()
        .collect();
        let furniture: HashMap<char, String> = [
            ('h', s("f_chair")),
            ('c', s("f_counter")),
            ('t', s("f_toilet")),
            ('b', s("f_bed")),
        ]
        .into_iter()
        .collect();

        let kind = |ch: char| cell_to_prop(ch, &terrain, &furniture).map(|(k, _)| k);
        assert_eq!(kind('w'), Some(PropKind::Wall));
        assert_eq!(kind('W'), Some(PropKind::Wall));
        assert_eq!(kind('^'), None); // gate skipped
        assert_eq!(kind('h'), Some(PropKind::Chair));
        assert_eq!(kind('c'), Some(PropKind::Table));
        assert_eq!(kind('b'), Some(PropKind::Furniture)); // bed
        assert_eq!(kind('t'), Some(PropKind::Furniture)); // toilet
        assert_eq!(kind('.'), None); // floor skipped
        assert_eq!(kind(' '), None); // unknown
        // Walls carry a material colour, and materials differ.
        assert!(cell_to_prop('w', &terrain, &furniture).unwrap().1.is_some());
        assert_ne!(wall_color("t_brick_wall"), wall_color("t_wall_log"));

        // A window becomes a translucent glass panel (its own kind),
        // tinted, sitting in the wall line.
        let win: HashMap<char, String> = [(':', s("t_window"))].into_iter().collect();
        assert_eq!(
            cell_to_prop(':', &win, &HashMap::new()),
            Some((PropKind::Window, Some(WINDOW_COLOR)))
        );
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
        let err = mapgen_to_template(GARAGE_JSON, "s_no_such_building", CDDA_TILE, 0).unwrap_err();
        assert_eq!(err, CddaError::NotFound("s_no_such_building".to_string()));
    }

    #[test]
    fn seeds_can_resolve_a_more_furnished_variant() {
        // At least one seed picks a variant palette (the hoarder) that
        // resolves visibly more furniture than the plainest one.
        let count = |seed: u32| {
            let t = assemble_building(HOUSE_JSON, "house_01", "house_01_roof", seed).unwrap();
            t.props
                .iter()
                .filter(|p| p.kind == PropKind::Furniture)
                .count()
        };
        let counts: Vec<usize> = (0..HOUSE_VARIANTS).map(count).collect();
        let (min, max) = (
            *counts.iter().min().unwrap(),
            *counts.iter().max().unwrap(),
        );
        assert!(max > min, "expected some variant to be more furnished: {counts:?}");
    }

    #[test]
    fn wall_colour_varies_across_house_seeds() {
        use std::collections::HashSet;
        let mut colors = HashSet::new();
        for seed in 0..HOUSE_VARIANTS {
            let t = assemble_building(HOUSE_JSON, "house_01", "house_01_roof", seed).unwrap();
            for p in &t.props {
                if matches!(p.kind, PropKind::Wall | PropKind::WallNS | PropKind::WallEW) {
                    if let Some(c) = p.color {
                        colors.insert(format!("{c:?}"));
                    }
                }
            }
        }
        assert!(
            colors.len() > 1,
            "expected walls of different materials/colours across seeds: {colors:?}"
        );
    }

    #[test]
    fn imports_palette_driven_house_with_walls_and_furniture() {
        // house_01 is fully palette-driven — walls + the domestic
        // furniture vocabulary come from resolving its 3 palettes.
        let t = house_template().expect("house should import");
        let n = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
        let walls = n(PropKind::Wall) + n(PropKind::WallNS) + n(PropKind::WallEW);
        assert!(walls > 10, "expected a resolved wall outline, got {walls}");
        let furniture = n(PropKind::Chair) + n(PropKind::Table) + n(PropKind::Furniture);
        assert!(furniture > 3, "expected resolved furniture, got {furniture}");
        assert!(n(PropKind::Roof) > 0, "expected a roof");
    }

    #[test]
    fn house_has_oriented_glass_windows() {
        // The house palettes place windows in the exterior walls; they
        // resolve to the translucent Window kind, oriented into thin
        // runs like the walls they sit in.
        let t = house_template().expect("house should import");
        let windows = t.props.iter().filter(|p| p.kind.is_window()).count();
        assert!(windows > 0, "expected glass windows in the house walls");
        assert!(
            t.props
                .iter()
                .any(|p| matches!(p.kind, PropKind::WindowNS | PropKind::WindowEW)),
            "windows should orient thin along their wall run"
        );
    }
}
