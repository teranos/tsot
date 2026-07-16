//! Canonical buildings the game ships — assembly (ground floor + roof)
//! and the startup registry the streamer picks from.

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
#[derive(Default)]
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

/// Parse every building we ship, once. Import failures are returned
/// alongside the templates (sacred — never dropped) so the consumer
/// can route them to its observability bus. Cdda-the-crate stays
/// framework-agnostic; game routes these to obs.
pub fn load_building_templates() -> (BuildingTemplates, Vec<String>) {
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
    let mut failures = Vec::new();
    for (name, result) in specs {
        match result {
            Ok(t) => templates.push(t),
            Err(e) => failures.push(format!("[cdda] {name} import failed: {e}")),
        }
    }
    let half_extents = templates.iter().map(footprint_half).collect();
    (BuildingTemplates { templates, half_extents }, failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::PropKind;

    /// Determinism property: every seeded building resolves to the same
    /// bytes every time it's resolved — the cross-peer invariant that
    /// keeps two players' worlds identical.
    ///
    /// Asserted as a PROPERTY (resolve twice, require equal), not a
    /// pinned snapshot. The guarantee is structural: the resolver is a
    /// pure function of the pinned CDDA corpus, so a given building can
    /// only ever produce one output. There are no hand-maintained
    /// expected digests to re-pin when the geometry legitimately changes
    /// — this scales to any number of assets untouched. (A single
    /// machine can't prove cross-*platform* agreement; that's a
    /// two-target comparison, not a value pinned from one machine. What
    /// this catches is a resolver that isn't pure: unseeded RNG, HashMap
    /// iteration order leaking into output, a time/address-dependent
    /// value.) See `template::Template::stable_digest`.
    #[test]
    fn every_shipped_building_resolves_deterministically() {
        let (a, _) = load_building_templates();
        let (b, _) = load_building_templates();
        assert!(!a.templates.is_empty(), "no buildings resolved at all");
        assert_eq!(
            a.templates.len(),
            b.templates.len(),
            "the number of resolved templates is itself nondeterministic: {} vs {}",
            a.templates.len(),
            b.templates.len()
        );
        for (i, (ta, tb)) in a.templates.iter().zip(b.templates.iter()).enumerate() {
            let (da, db) = (ta.stable_digest(), tb.stable_digest());
            assert_eq!(
                da, db,
                "template [{i}] resolves nondeterministically across two independent \
                 loads of the pinned corpus: 0x{da:016X} vs 0x{db:016X} — the resolver \
                 is not a pure function of its input (unseeded RNG, HashMap iteration \
                 order leaking into output, or a time/address-dependent value)"
            );
        }
    }

    #[test]
    fn building_templates_load_the_garage() {
        let (t, _) = load_building_templates();
        assert!(!t.is_empty(), "garage should parse");
        assert!(t.templates[0].props.len() > 50, "garage should be many props");
        assert_eq!(t.templates.len(), t.half_extents.len(), "extents track templates");
    }

    #[test]
    fn imports_garage_with_oriented_walls_and_roof() {
        let t = garage_template().expect("garage should import");
        let n = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
        let walls = n(PropKind::Wall) + n(PropKind::WallNS) + n(PropKind::WallEW);
        assert!(walls > 10, "expected a wall outline, got {walls}");
        assert!(
            n(PropKind::WallNS) + n(PropKind::WallEW) > 0,
            "walls should be oriented into thin runs"
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
    fn imports_palette_driven_house_with_walls_and_roof() {
        // house_01 is fully palette-driven — the wall outline comes from
        // resolving its 3 palettes (not inline terrain).
        let t = house_template().expect("house should import");
        let n = |k: PropKind| t.props.iter().filter(|p| p.kind == k).count();
        let walls = n(PropKind::Wall) + n(PropKind::WallNS) + n(PropKind::WallEW);
        assert!(walls > 10, "expected a resolved wall outline, got {walls}");
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
            half > crate::CDDA_TILE * 18.0,
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
