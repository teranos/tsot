//! Campfire — warm fire at the western edge of the world, planted on
//! the `Pin::Campfire` anchor from `map.rs`. Log pile + emissive
//! flame `Cone` + orange `PointLight`, with a slow low-frequency
//! flicker on both the light intensity and the flame material's
//! emissive so the fire doesn't read as a static bulb.
//!
//! This is rave's first zone that ships with its content already
//! parented to a `Pin` from day one — the pattern the existing
//! zones (Stage, Dancefloor, BarZone, Trail) migrate to on
//! subsequent commits. Moving the whole campfire is
//! `pin.transform.translation += ...` on the anchor; logs, flame,
//! and light follow.

use bevy::prelude::*;

use crate::map::Pin;

/// Marker on the flame Cone so `flicker_fire` can look up its
/// material handle to modulate the emissive each frame.
#[derive(Component)]
pub struct CampfireFlame;

/// Marker on the fire's `PointLight` so `flicker_fire` can modulate
/// its intensity without also touching every other light in the scene.
#[derive(Component)]
pub struct CampfireLight;

/// Baseline PointLight intensity — `flicker_fire` scales this by
/// `1 ± ~0.25` each frame.
const BASE_INTENSITY: f32 = 1_500_000.0;

/// Baseline flame emissive RGB. `flicker_fire` multiplies R and G in
/// lockstep with the light so the flame surface tracks brightness.
const BASE_EMISSIVE_R: f32 = 3.0;
const BASE_EMISSIVE_G: f32 = 1.2;
const BASE_EMISSIVE_B: f32 = 0.2;

pub fn setup_campfire(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    pins: Query<(Entity, &Pin)>,
) {
    let Some((pin_entity, _)) = pins.iter().find(|(_, p)| matches!(p, Pin::Campfire))
    else {
        // Campfire pin not spawned. `setup_campfire` is ordered
        // `.after(map::setup_map)` in `lib.rs` so this shouldn't
        // happen; if it does, better to no-op than to spawn logs
        // floating unparented at the world origin.
        return;
    };

    let log_mesh = meshes.add(Cuboid::new(4.0, 4.0, 30.0));
    let log_mat = materials.add(StandardMaterial::from(Color::srgb(0.15, 0.09, 0.05)));

    let flame_mesh = meshes.add(Cone {
        radius: 12.0,
        height: 28.0,
    });
    let flame_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.4, 0.05),
        emissive: LinearRgba::new(BASE_EMISSIVE_R, BASE_EMISSIVE_G, BASE_EMISSIVE_B, 1.0),
        ..default()
    });

    commands.entity(pin_entity).with_children(|parent| {
        // Three logs crossed at 60° around Y. Base at y=2 so the
        // bottoms touch the ground.
        for angle_deg in [0.0_f32, 60.0, 120.0] {
            parent.spawn((
                Mesh3d(log_mesh.clone()),
                MeshMaterial3d(log_mat.clone()),
                Transform::from_xyz(0.0, 2.0, 0.0)
                    .with_rotation(Quat::from_rotation_y(angle_deg.to_radians())),
            ));
        }

        // Flame cone above the logs, tip pointing up.
        parent.spawn((
            CampfireFlame,
            Mesh3d(flame_mesh.clone()),
            MeshMaterial3d(flame_mat.clone()),
            Transform::from_xyz(0.0, 16.0, 0.0),
        ));

        // Warm PointLight just above the flame — casts on the logs
        // and any nearby trees. Shadows off to match the rest of
        // rave's lighting (see `floorplan.rs` lights).
        parent.spawn((
            CampfireLight,
            PointLight {
                color: Color::srgb(1.0, 0.55, 0.2),
                intensity: BASE_INTENSITY,
                range: 400.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 30.0, 0.0),
        ));
    });
}

pub fn flicker_fire(
    time: Res<Time>,
    mut lights: Query<&mut PointLight, With<CampfireLight>>,
    flames: Query<&MeshMaterial3d<StandardMaterial>, With<CampfireFlame>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = time.elapsed_secs();
    // Two-sine low-frequency noise, sum ∈ ≈ [-1.5, 1.5], divided
    // to land ≈ ±0.5, then scaled to ±0.25 amplitude on the
    // multiplier. Softer than the strobes' hard-clipped sine.
    let noise = ((t * 3.7).sin() + (t * 7.3).sin() * 0.5) / 3.0;
    let modulator = 1.0 + noise * 0.5;

    for mut light in &mut lights {
        light.intensity = BASE_INTENSITY * modulator;
    }
    for flame_mat_handle in &flames {
        if let Some(mut mat) = materials.get_mut(&flame_mat_handle.0) {
            mat.emissive = LinearRgba::new(
                BASE_EMISSIVE_R * modulator,
                BASE_EMISSIVE_G * modulator,
                BASE_EMISSIVE_B,
                1.0,
            );
        }
    }
}
