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
/// The house mapgen — palette-driven (CC-BY-SA 3.0, CDDA).
const HOUSE_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cdda/house01.json"));
/// A shed — CDDA has no standalone one, so this is an original inline
/// mapgen in the same format (ours, so it stays vendored in-tree).
const SHED_JSON: &str = include_str!("../../assets/buildings/shed.json");

/// How many hash-varied house variants to pre-build. Each picks its
/// own variant palette (standard / abandoned / hoarder / survivor) +
/// fence/wall/lino, so building_index lands on visibly different houses.
pub const HOUSE_VARIANTS: u32 = 6;

/// The garage, imported from the embedded CDDA mapgen — ground floor
/// (walls + furniture) plus its roof z-level capping the building.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::PropKind;

    #[test]
    fn building_templates_load_the_garage() {
        let t = load_building_templates();
        assert!(!t.0.is_empty(), "garage should parse");
        assert!(t.0[0].props.len() > 50, "garage should be many props");
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
