//! CDDA seam — the boundary between Cataclysm: Dark Days Ahead's
//! authored mapgen JSON and our world. Parse + palette resolve +
//! placement rules + building assembly, producing `Template` values a
//! consumer can stamp into an ECS.
//!
//! Split by concern:
//! - [`template`]: the pure `Prop` / `PropKind` / `Template` types +
//!   `resolve_placements`, `rotate_template`, `stable_digest`. No ECS,
//!   no RNG. This is the wire shape crossing the seam.
//! - [`parse`]: JSON entry types + walk helpers.
//! - [`cells`]: raw CDDA char → prop-vocabulary mapping.
//! - [`palette`]: nested-palette resolver with per-building parameter
//!   seed (variety without RNG).
//! - [`placement`]: mapgen → `Template` (edge-placed walls, flood-fill
//!   interior detection, roof pass).
//! - [`building`]: canonical building assembly + startup registry.
//! - [`chunks`]: chunk-level placement, index, rotation.
//!
//! CDDA corpus is a pinned build-time dependency, never vendored. See
//! this crate's `RELEASE`, `COMMIT`, `files.txt`, `build.rs`, and
//! `ATTRIBUTION.md` (CC-BY-SA 3.0 CDDA content).

pub mod building;
pub mod cells;
pub mod chunks;
pub(crate) mod hash;
pub mod palette;
pub mod parse;
pub mod placement;
pub mod template;

pub use building::{
    BuildingTemplates, HOUSE_VARIANTS, assemble_building, garage_template, house_template,
    load_building_templates, shed_template,
};
pub use chunks::{
    BUILDING_FOOTPRINT_HALF, building_anchor_in_chunk, building_index, building_rotation,
};
pub use placement::{CDDA_TILE, CddaError, ROOF_HEIGHT, mapgen_to_template, roof_to_props};
pub use template::{
    Prop, PropKind, Template, TreeKind, TreePlacement, resolve_placements, rotate_template,
};
