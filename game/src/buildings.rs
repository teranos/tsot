//! Bevy `Resource` wrapper around `cdda::BuildingTemplates`. The cdda
//! crate stays framework-agnostic (no Bevy derive across the seam);
//! game re-shapes its output into a Bevy resource so systems can query
//! it via `Res<_>`.

use bevy_ecs::prelude::*;

#[derive(Resource, Default)]
pub struct BuildingTemplates {
    pub templates: Vec<cdda::Template>,
    pub half_extents: Vec<f32>,
}

impl BuildingTemplates {
    /// Load every shipped building through the cdda seam and re-shape
    /// into the Bevy resource. Import failures are returned alongside
    /// (sacred — the caller routes to obs).
    pub fn load() -> (Self, Vec<String>) {
        let (b, failures) = cdda::load_building_templates();
        (
            Self {
                templates: b.templates,
                half_extents: b.half_extents,
            },
            failures,
        )
    }

    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }

    pub fn len(&self) -> usize {
        self.templates.len()
    }
}
