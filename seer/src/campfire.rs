// Campfire — ported from rave/src/campfire.rs, stripped of Bevy's PBR
// dependency (StandardMaterial, PointLight, Mesh3d) which seer
// deliberately doesn't ship. The ECS shape (marker component + flicker
// system) survives; visuals surface through seer's own render path,
// where the fire lands as one instance whose color scales with the
// current flicker intensity.
//
// Rave's version parented three logs + a flame Cone + a warm
// PointLight to a Pin::Campfire anchor. Seer has neither pins nor
// point lights: the fire is a single AabbCollider'd cube at a fixed
// world position, and the flicker modulates its instance color when
// render.rs picks it up.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::physics::{AabbCollider, Position};

/// Marker + per-fire state. `intensity` is a unitless multiplier
/// around 1.0 that `flicker_fire` mutates each tick. Rendered as a
/// warm-orange cube whose color scales with intensity.
#[derive(Component)]
pub struct Campfire {
    pub intensity: f32,
}

/// Where the fire sits — western edge of the walkable floor, off the
/// player's spawn line so it's visible in the render but not in the
/// way of movement.
pub const SPAWN_POS: Vec3 = Vec3::new(-1500.0, 20.0, 0.0);

/// Neutral baseline. `flicker_fire` scales this by `1 ± 0.25`.
pub const BASE_INTENSITY: f32 = 1.0;

/// Half-extents of the invisible AABB the player bounces off.
pub const COLLIDER_HALF: Vec3 = Vec3::new(15.0, 15.0, 15.0);

/// Approximate seconds-per-frame assumed by `flicker_fire`. seer
/// doesn't ship `bevy_time` (that pulls platform glue), so the
/// flicker uses a synthetic tick count instead of wall-clock. 60 FPS
/// is the placeholder — CI ticks 180 frames = ~3 simulated seconds
/// of flicker.
pub const TICK_SECONDS: f32 = 1.0 / 60.0;

/// Two-sine low-frequency noise → intensity multiplier. Ported
/// verbatim from rave's `flicker_fire`:
///
/// ```text
/// noise     = (sin(3.7 t) + 0.5 sin(7.3 t)) / 3
/// modulator = 1 + noise * 0.5
/// ```
///
/// Pure fn — testable given `t` in seconds. Bounded: `|noise| ≤ 0.5`,
/// so `modulator ∈ [0.75, 1.25]`.
pub fn flicker_modulator(t: f32) -> f32 {
    let noise = ((t * 3.7).sin() + (t * 7.3).sin() * 0.5) / 3.0;
    1.0 + noise * 0.5
}

pub fn setup_campfire(mut commands: Commands) {
    commands.spawn((
        Campfire {
            intensity: BASE_INTENSITY,
        },
        Position(SPAWN_POS),
        AabbCollider {
            half_extents: COLLIDER_HALF,
        },
    ));
}

/// System — advances the flicker by one tick per invocation and
/// writes the resulting intensity back to every Campfire. `Local<f32>`
/// carries the synthetic elapsed-time state per-system, so removing
/// bevy_time from the crate doesn't require a global Resource.
pub fn flicker_fire(mut fires: Query<&mut Campfire>, mut t: Local<f32>) {
    *t += TICK_SECONDS;
    let m = flicker_modulator(*t);
    for mut fire in &mut fires {
        fire.intensity = BASE_INTENSITY * m;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modulator_at_zero_is_one() {
        assert!((flicker_modulator(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn modulator_stays_bounded() {
        // Sample the full range at a fine step; the bound is derived
        // analytically as [0.75, 1.25] but a numerical check catches
        // any accidental amplitude edit.
        for i in 0..10_000 {
            let t = i as f32 * 0.01;
            let m = flicker_modulator(t);
            assert!(
                (0.75..=1.25).contains(&m),
                "flicker_modulator({t}) = {m} out of range"
            );
        }
    }

    #[test]
    fn modulator_is_deterministic() {
        // Same t → same m. Guards against any hidden global state
        // creeping into the fn (e.g. someone reaching for rand::).
        let t = 1.7;
        let a = flicker_modulator(t);
        let b = flicker_modulator(t);
        assert_eq!(a, b);
    }
}
