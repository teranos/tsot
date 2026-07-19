//! game/src/tune — runtime-tunable knobs for the tree renderer.
//!
//! Every value that used to be a `const` scattered across tree_surface,
//! tree_emit, and the wind shader lives here in one mutable struct.
//! An in-game HUD panel (`tune_hud.rs`) writes to it; every subsystem
//! reads via `tune::get()` each snapshot.
//!
//! Setting any wood-shape field invalidates the per-species mesh cache
//! (`tree_surface::invalidate_species_cache`) so the next frame
//! regenerates. Non-wood fields (wind, leaf) are read live every frame
//! and need no invalidation.
//!
//! No peer-sync: wood mesh is purely visual, so local tuning stays
//! local. Two peers running different tunings just see slightly
//! different oaks.
//!
//! Defaults match the pre-tune constants — turning tuning ON changes
//! nothing until something writes a value.

use std::cell::RefCell;

/// One coherent bag of tunables. Grouped by subsystem for the HUD
/// panel layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TuneParams {
    // -------- Wood shape --------
    /// `voxel target = trunk_radius * this`. Smaller → higher res →
    /// smoother thin trunks and finer twig detail.
    pub wood_voxel_ratio: f32,
    /// `rfloor = voxel * this`. Larger fattens all limbs; too low and
    /// thin twigs vanish between grid lines.
    pub wood_rfloor_coeff: f32,
    /// `blend = voxel * this`. Smin fillet at forks. Larger → more
    /// organic melt; too large → mushroom cap at the crown.
    pub wood_blend_coeff: f32,
    /// AABB padding around cones (multiples of trunk_radius).
    pub wood_aabb_pad: f32,
    /// Resolution clamp for the marching-tet grid.
    pub wood_res_min: u32,
    pub wood_res_max: u32,

    // -------- Roots (buttress flare below the ankle) --------
    /// Root cone A end: `y = trunk_radius * this`.
    pub root_ankle_y: f32,
    /// Root cone B end horizontal: `r = trunk_radius * this`.
    pub root_reach: f32,
    /// Root cone B end vertical: `y = -trunk_radius * this`.
    pub root_depth: f32,
    /// Root cone A radius: `trunk_radius * this`.
    pub root_ra_mult: f32,
    /// Root cone B radius: `trunk_radius * this`.
    pub root_rb_mult: f32,

    // -------- Wind --------
    /// Wood sway weight — the whole wood bends by this × pos.y × amp.
    /// 0 = rigid; 0.2 default (a stiff column).
    pub wind_wood_sway: f32,
    /// Global wind amplitude in world units (matches shader `WIND_AMP`).
    pub wind_amp: f32,
    /// Wind temporal frequency multiplier. 1.0 = baseline.
    pub wind_speed: f32,
    /// Leaf sway multiplier — leaves already sway by `i_axis.w`; this
    /// scales it further.
    pub wind_leaf_mult: f32,

    // -------- Leaves --------
    /// Multiplier on species `leaves_per_tip`.
    pub leaf_density_mult: f32,
    /// Multiplier on species `leaf_element_ratio`.
    pub leaf_size_mult: f32,
    /// Multiplier on species `cluster_radius_ratio`.
    pub leaf_cluster_mult: f32,
    /// Multiplier on species `autumn`. 0 = evergreen; >1 = more turn.
    pub autumn_mult: f32,
    /// Multiplier on species `dead_limb_odds`. 0 = no deadwood.
    pub deadwood_mult: f32,
}

impl TuneParams {
    /// Compiled-in defaults — every value matches what the old `const`
    /// said. Boot state: as if tuning didn't exist.
    pub const fn defaults() -> Self {
        Self {
            wood_voxel_ratio: 0.4,
            wood_rfloor_coeff: 0.5,
            wood_blend_coeff: 0.5,
            wood_aabb_pad: 2.0,
            wood_res_min: 40,
            wood_res_max: 192,
            root_ankle_y: 1.5,
            root_reach: 7.0,
            root_depth: 3.5,
            root_ra_mult: 1.15,
            root_rb_mult: 0.35,
            wind_wood_sway: 0.2,
            wind_amp: 5.0,
            wind_speed: 1.0,
            wind_leaf_mult: 1.0,
            leaf_density_mult: 1.0,
            leaf_size_mult: 1.0,
            leaf_cluster_mult: 1.0,
            autumn_mult: 1.0,
            deadwood_mult: 1.0,
        }
    }
}

thread_local! {
    static TUNE: RefCell<TuneParams> = const { RefCell::new(TuneParams::defaults()) };
}

/// Read the current tuning. Cheap — one copy of a small struct.
pub fn get() -> TuneParams {
    TUNE.with(|c| *c.borrow())
}

/// Replace the tuning wholesale. Callers changing wood-shape fields
/// must also invalidate `tree_surface::species_wood_mesh` so meshes
/// regenerate; the HUD does this after every commit.
pub fn set(params: TuneParams) {
    TUNE.with(|c| *c.borrow_mut() = params);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_const() {
        const D: TuneParams = TuneParams::defaults();
        assert_eq!(D.wood_voxel_ratio, 0.4);
        assert_eq!(D.wind_wood_sway, 0.2);
    }

    #[test]
    fn set_and_get_round_trip() {
        let mut p = TuneParams::defaults();
        p.wind_amp = 42.0;
        set(p);
        assert_eq!(get().wind_amp, 42.0);
        set(TuneParams::defaults());
    }
}
