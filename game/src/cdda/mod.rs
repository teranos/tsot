//! CDDA mapgen importer — parse a Cataclysm: Dark Days Ahead mapgen
//! JSON entry into a `Template` the stamp machinery can place.
//!
//! Split by concern:
//! - [`parse`]: JSON entry types + walk helpers.
//! - [`cells`]: raw CDDA char → prop-vocabulary mapping.
//! - [`placement`]: mapgen → `Template` (edge-placed walls, flood-fill
//!   interior detection, roof pass).
//! - [`building`]: canonical building assembly + startup registry.
//! - [`chunks`]: chunk-level placement, index, rotation.
//!
//! MVP scope: inline-palette buildings + a few palette-driven ones,
//! single z-level with a matching roof mapgen. CDDA terrain/furniture
//! ids map into the prop vocabulary — walls/fences → Wall, chairs →
//! Chair, counters/desks/tables → Table — everything else is skipped.
//!
//! The garage layout under `assets/cdda/` is CC-BY-SA 3.0 CDDA content —
//! see `assets/cdda/ATTRIBUTION.md`.

pub mod building;
pub mod cells;
pub mod chunks;
pub mod parse;
pub mod placement;

pub use building::{
    BuildingTemplates, HOUSE_VARIANTS, assemble_building, garage_template, house_template,
    load_building_templates, shed_template,
};
pub use chunks::{
    BUILDING_FOOTPRINT_HALF, building_anchor_in_chunk, building_index, building_rotation,
};
pub use placement::{CDDA_TILE, CddaError, ROOF_HEIGHT, mapgen_to_template, roof_to_props};
