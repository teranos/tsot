//! Canonical buildings the game ships — assembly (ground floor + roof)
//! and the startup registry the streamer picks from.

use bevy_ecs::prelude::*;

use crate::obs;
use crate::template::Template;

use super::placement::{CDDA_TILE, CddaError, ROOF_HEIGHT, mapgen_to_template, roof_to_props};

// CDDA mapgen embedded from the build-time corpus (build.rs copies it
// out of the release pinned in CDDA_RELEASE — never vendored in git).
// CC-BY-SA 3.0, CleverRaven / CDDA; see assets/cdda/ATTRIBUTION.md.
const GARAGE_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/garage.json"));
/// The house mapgens — palette-driven (CC-BY-SA 3.0, CDDA). All three
/// reference the same palette set, so they resolve from the palettes we
/// already fetch; each is a distinct authored floor plan.
const HOUSE01_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/house01.json"));
const HOUSE02_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/house02.json"));
const HOUSE03_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/house03.json"));
const HOUSE04_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/house04.json"));
/// The daycare — a single-tile civic building. Its walls + windows are
/// defined inline in the mapgen, so it needs no palette beyond the roof
/// palette we already fetch.
const DAYCARE_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/daycare.json"));
/// The school — a 3×3 *multi-tile* building (72×72), driven by
/// `school_palette` which it declares inline (registered by the palette
/// resolver). Streams via the grid-sliced per-chunk path in `chunk.rs`.
const SCHOOL_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/school_1.json"));
/// A shed — CDDA has no standalone one, so this is an original inline
/// mapgen in the same format (ours, so it stays vendored in-tree).
const SHED_JSON: &str = include_str!("../../assets/buildings/shed.json");

/// Every shipped mapgen JSON. Exposed so the palette resolver can
/// register palettes a building declares *inline* in its own file —
/// CDDA registers all `type: palette` objects globally, wherever
/// they're declared (e.g. the school's `school_palette`).
pub(crate) const SHIPPED_MAPGEN: &[&str] = &[
    GARAGE_JSON,
    HOUSE01_JSON,
    HOUSE02_JSON,
    HOUSE03_JSON,
    HOUSE04_JSON,
    DAYCARE_JSON,
    SCHOOL_JSON,
    SHED_JSON,
];

/// The palette-driven house layouts the world ships: `(json, ground om,
/// roof om)`. Each is built into `HOUSE_VARIANTS` seeded variants at
/// load. Adding another CDDA house = one line here + one in
/// `cdda-files.txt`.
const HOUSE_LAYOUTS: &[(&str, &str, &str)] = &[
    (HOUSE01_JSON, "house_01", "house_01_roof"),
    (HOUSE02_JSON, "house_02", "house_02_roof"),
    (HOUSE03_JSON, "house_03", "house_03_roof"),
    (HOUSE04_JSON, "house_04", "house_04_roof"),
];

/// How many hash-varied house variants to pre-build. Each picks its
/// own variant palette (standard / abandoned / hoarder / survivor) +
/// fence/wall/lino, so building_index lands on visibly different houses.
pub const HOUSE_VARIANTS: u32 = 6;

/// The garage, imported from the embedded CDDA mapgen — ground floor
/// (walls + furniture) plus its roof z-level capping the building.
pub fn garage_template() -> Result<Template, CddaError> {
    assemble_building(GARAGE_JSON, "s_garage_1", "s_garage_roof_1", 0)
}

/// house_01 at the canonical seed 0 (used by tests). The world uses the
/// seeded variants of every layout, built in `load_building_templates`.
pub fn house_template() -> Result<Template, CddaError> {
    assemble_building(HOUSE01_JSON, "house_01", "house_01_roof", 0)
}

/// A small shed (original inline mapgen, no palettes).
pub fn shed_template() -> Result<Template, CddaError> {
    assemble_building(SHED_JSON, "shed_1", "shed_roof", 0)
}

/// The daycare — inline walls/windows, one flat roof; no seeded variants
/// (its only palette is cosmetic carpets).
pub fn daycare_template() -> Result<Template, CddaError> {
    assemble_building(DAYCARE_JSON, "s_daycare", "s_daycare_roof", 0)
}

/// The school — its 3×3 ground floor (`school_1_*`) capped by the roof
/// z-level (`school_4_*`). The upper floors (`school_2_*`/`school_3_*`)
/// are z-levels we don't place yet. One big 72×72 grid → a template that
/// spans several chunks, streamed per-chunk (see `chunk.rs`).
pub fn school_template() -> Result<Template, CddaError> {
    assemble_building(SCHOOL_JSON, "school_1_1", "school_4_5", 0)
}

/// Ground floor (walls + furniture, palettes resolved at `seed`) + roof.
pub fn assemble_building(
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
pub struct BuildingTemplates {
    pub templates: Vec<Template>,
    /// Per-template footprint half-extent — the largest `|offset.x|` or
    /// `|offset.z|` over its props, i.e. how far the building reaches
    /// from its anchor. Rotation only swaps the axes, so this max is
    /// rotation-safe. Streaming uses it to distribute a building's props
    /// across the chunks they land in (multi-tile support).
    pub half_extents: Vec<f32>,
}

impl BuildingTemplates {
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }
    pub fn len(&self) -> usize {
        self.templates.len()
    }
}

/// The footprint half-extent of one template (max `|x|`/`|z|` offset).
fn footprint_half(t: &Template) -> f32 {
    t.props
        .iter()
        .fold(0.0_f32, |m, p| m.max(p.offset.x.abs()).max(p.offset.z.abs()))
}

/// Parse every building we ship, once. Import failures surface on the
/// obs bus (sacred); the building simply won't appear.
pub fn load_building_templates() -> BuildingTemplates {
    let mut specs: Vec<(String, Result<Template, CddaError>)> = vec![
        ("garage".to_string(), garage_template()),
        ("shed".to_string(), shed_template()),
        ("daycare".to_string(), daycare_template()),
        ("school".to_string(), school_template()),
    ];
    // Every house layout × every palette seed — so the streamer lands on
    // different floor plans AND different material/furniture variants.
    for (json, ground, roof) in HOUSE_LAYOUTS {
        for seed in 0..HOUSE_VARIANTS {
            specs.push((
                format!("{ground}#{seed}"),
                assemble_building(json, ground, roof, seed),
            ));
        }
    }
    let mut templates = Vec::new();
    for (name, result) in specs {
        match result {
            Ok(t) => templates.push(t),
            Err(e) => obs::emit(&format!("[cdda] {name} import failed: {e}")),
        }
    }
    let half_extents = templates.iter().map(footprint_half).collect();
    BuildingTemplates { templates, half_extents }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::PropKind;

    /// Golden-master: every seeded building resolves to the same bytes
    /// on every peer. If this test fires, either the placement pipeline
    /// changed (edge shifts, palette resolver, flood-fill…) or the
    /// PropKind tags moved — read the reported mismatch and confirm the
    /// change was intentional before updating the pinned digest.
    /// See `template::Template::stable_digest`.
    #[test]
    fn every_shipped_building_digests_stably() {
        let t = load_building_templates();
        let actual: Vec<u64> =
            t.templates.iter().map(|t| t.stable_digest()).collect();
        // Pinned via the first canonical run of `load_building_templates`
        // — 4 one-offs (garage, shed, daycare, school) then 4 house
        // layouts × 6 palette seeds = 28 templates.
        const EXPECTED: &[u64] = &[
            0xB84F97743A8A6D00, // garage
            0x6AC0D34AA85B4827, // shed
            0x725F7482D072E2F0, // daycare
            0x15F966A8691475BF, // school
            0x41CC62D5A16D698D, // house_01 seed 0
            0x719685D6400B7D47, // house_01 seed 1
            0x2227BE6134DEB4D7, // house_01 seed 2
            0x719685D6400B7D47, // house_01 seed 3
            0x8B0F90E993B324DB, // house_01 seed 4
            0xCBD298AFF65F326F, // house_01 seed 5
            0x534CDA273342CA22, // house_02 seed 0
            0x8F8CFF1B45407F68, // house_02 seed 1
            0xD94C1747ACDBDD8D, // house_02 seed 2
            0x2E6F2F6DCEB18EE4, // house_02 seed 3
            0xD54E5578751820CD, // house_02 seed 4
            0xD2EE431E0CCF8CF1, // house_02 seed 5
            0x7C036E90E5159CEA, // house_03 seed 0
            0x31FFBF37D123D7B0, // house_03 seed 1
            0x108ACEF8748CF12E, // house_03 seed 2
            0x31FFBF37D123D7B0, // house_03 seed 3
            0x7FEAA65E6CE6B9FC, // house_03 seed 4
            0x6B97CF8F6C65C08E, // house_03 seed 5
            0x87308BCF54C0C31F, // house_04 seed 0
            0x62C0AAC84A2D7BF0, // house_04 seed 1
            0xED8A849C92393512, // house_04 seed 2
            0x62C0AAC84A2D7BF0, // house_04 seed 3
            0xE06A2D2B0C7AEA94, // house_04 seed 4
            0x8D87FD212FE5B8BE, // house_04 seed 5
        ];
        if actual != EXPECTED {
            // Errors are sacred (ERROR.md #5): don't just say "drifted" and
            // dump 28 opaque hashes — name WHICH building changed and show
            // expected-vs-got, so the reader knows exactly what to verify
            // before re-pinning. Labels track the canonical order
            // load_building_templates emits: 4 one-offs, then each house
            // layout × 6 palette seeds.
            let labels: Vec<String> = {
                let mut v: Vec<String> =
                    ["garage", "shed", "daycare", "school"].iter().map(|s| s.to_string()).collect();
                for h in 1..=4 {
                    for seed in 0..6 {
                        v.push(format!("house_{h:02} seed {seed}"));
                    }
                }
                v
            };
            let mut drifted = Vec::new();
            for i in 0..actual.len().max(EXPECTED.len()) {
                let (e, a) = (EXPECTED.get(i), actual.get(i));
                if e == a {
                    continue;
                }
                let label = labels.get(i).map(String::as_str).unwrap_or("<template beyond the pinned set>");
                match (e, a) {
                    (Some(e), Some(a)) => {
                        drifted.push(format!("  [{i}] {label}: expected 0x{e:016X}, got 0x{a:016X}"))
                    }
                    (Some(e), None) => drifted
                        .push(format!("  [{i}] {label}: expected 0x{e:016X}, MISSING (template count shrank)")),
                    (None, Some(a)) => drifted
                        .push(format!("  [{i}] {label}: UNEXPECTED extra template, got 0x{a:016X}")),
                    (None, None) => {}
                }
            }
            let dump: Vec<String> =
                actual.iter().map(|d| format!("0x{d:016X}")).collect();
            panic!(
                "building digests drifted from the golden master — {} of {} templates changed.\n\
                 WHICH changed (confirm EACH is an intended geometry/palette change before re-pinning):\n{}\n\n\
                 If every change above is intended, update EXPECTED to:\n[\n  {},\n]",
                drifted.len(),
                actual.len().max(EXPECTED.len()),
                drifted.join("\n"),
                dump.join(",\n  ")
            );
        }
    }

    #[test]
    fn building_templates_load_the_garage() {
        let t = load_building_templates();
        assert!(!t.is_empty(), "garage should parse");
        assert!(t.templates[0].props.len() > 50, "garage should be many props");
        assert_eq!(t.templates.len(), t.half_extents.len(), "extents track templates");
    }

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
    fn seeds_can_resolve_a_more_furnished_variant() {
        // At least one seed picks a variant palette (the hoarder) that
        // resolves visibly more furniture than the plainest one.
        let count = |seed: u32| {
            let t = assemble_building(HOUSE01_JSON, "house_01", "house_01_roof", seed).unwrap();
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
            let t = assemble_building(HOUSE01_JSON, "house_01", "house_01_roof", seed).unwrap();
            for p in &t.props {
                if matches!(p.kind, PropKind::Wall | PropKind::WallNS | PropKind::WallEW)
                    && let Some(c) = p.color
                {
                    colors.insert(format!("{c:?}"));
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
    fn every_house_layout_imports_with_walls_roof_and_windows() {
        // The safety net for adding CDDA houses: each shipped layout must
        // resolve to an enclosed, roofed, windowed building. A layout
        // that references a palette we don't fetch, or leans on
        // unhandled mapgen, fails here instead of silently spawning a
        // ruin in the world.
        for (json, ground, roof) in HOUSE_LAYOUTS {
            let t = assemble_building(json, ground, roof, 0)
                .unwrap_or_else(|e| panic!("{ground} should import: {e}"));
            let n = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
            let walls = n(PropKind::Wall) + n(PropKind::WallNS) + n(PropKind::WallEW);
            assert!(walls > 10, "{ground}: expected a wall outline, got {walls}");
            assert!(n(PropKind::Roof) > 0, "{ground}: expected a roof");
            assert!(
                t.props.iter().any(|p| p.kind.is_window()),
                "{ground}: expected glass windows"
            );
        }
    }

    #[test]
    fn school_is_a_multi_tile_enclosed_roofed_building() {
        // The multi-tile payoff: a 72×72 school resolves (via its inline
        // school_palette) into a big enclosed, roofed building whose
        // footprint spans well beyond a single 24-tile om.
        let t = school_template().expect("school should import");
        let n = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
        let walls = n(PropKind::Wall) + n(PropKind::WallNS) + n(PropKind::WallEW);
        assert!(walls > 30, "school: expected a large wall outline, got {walls}");
        assert!(n(PropKind::Roof) > 100, "school: expected a big roof, got {}", n(PropKind::Roof));
        let half = t
            .props
            .iter()
            .fold(0.0_f32, |m, p| m.max(p.offset.x.abs()).max(p.offset.z.abs()));
        assert!(
            half > crate::cdda::CDDA_TILE * 18.0,
            "school should span multiple om tiles, half-extent={half}"
        );
    }

    #[test]
    fn daycare_imports_with_walls_roof_and_windows() {
        // Single-tile civic building; inline walls/windows, so it must
        // resolve to an enclosed, roofed, windowed building from the
        // roof palette alone (its carpet palette isn't fetched).
        let t = daycare_template().expect("daycare should import");
        let n = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
        let walls = n(PropKind::Wall) + n(PropKind::WallNS) + n(PropKind::WallEW);
        assert!(walls > 10, "daycare: expected a wall outline, got {walls}");
        assert!(n(PropKind::Roof) > 0, "daycare: expected a roof");
        assert!(
            t.props.iter().any(|p| p.kind.is_window()),
            "daycare: expected glass windows"
        );
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
