//! The rave clearing — DJ booth, speakers, bar, dancefloor, truss,
//! strobes. Open-air: no walls, no roof, no toilets, no garderobe.
//! Sits at the origin of a much larger forest world (`room::FLOOR_HALF`);
//! the player walks in from the south along the trail in `trail.rs`.
//!
//! Coordinate system (camera looks roughly down from +Z, +Y):
//!   north (DJ + stage)  = -Z       south (clearing edge) = +Z
//!   west  (bar)         = -X       east  (open)          = +X
//!
//! All meshes are simple primitives (Cuboid + Plane3d). Humanoid
//! avatars + assets-from-CDN land in later slices.

use bevy::prelude::*;

use crate::physics::AabbCollider;

/// Half-extent of the clearing — the bounding region within which
/// the rave's physical structures sit. Trees in `trees.rs` are
/// excluded from this radius so the dancefloor isn't overgrown.
pub const CLEARING_HALF: f32 = 500.0;

const DANCEFLOOR_HALF: f32 = 160.0;

/// Strobe runtime mode. R24 default is `Off` — strobes only fire when
/// activated (R30 will add the click-to-cycle interaction).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StrobeMode {
    /// Light is dark, fixture is dark. No flash.
    Off,
    /// Sinusoid-driven flash, white only.
    Strobing,
}

/// Marker on each strobe PointLight so the animation system can find
/// them without grabbing every PointLight in the world.
#[derive(Component)]
pub struct Strobe {
    /// Phase offset in seconds — different per strobe so the four
    /// don't pulse in lockstep.
    pub phase: f32,
    /// Pulse frequency in Hz.
    pub frequency: f32,
    /// Whether the strobe is currently active. Changeable at runtime
    /// (R30 click-to-cycle); per-strobe so the user can light one
    /// corner without lighting all four.
    pub mode: StrobeMode,
    /// Material handle on the fixture mesh — `pulse_strobes` flips its
    /// emissive in lockstep with the light intensity so the source is
    /// visible, not just the surface bounce.
    pub fixture_material: Handle<StandardMaterial>,
}

/// Truss-mounted moving spotlight. Animation is `pulse_truss_lights`:
/// `yaw_amp` radians of sweep across the dancefloor, hue cycles around
/// a full circle every `color_period_s` seconds.
#[derive(Component)]
pub struct TrussSpot {
    pub phase: f32,
    pub yaw_freq: f32,
    pub yaw_amp: f32,
    pub color_period_s: f32,
    pub hue_offset: f32,
}

/// Marker on the speaker entities so the audio system can find the
/// two speakers without grabbing every cuboid in the world.
#[derive(Component)]
pub struct Speaker;

pub fn setup_floor_plan(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // ----- DJ booth + speakers (north end of the clearing) -----
    let dj_mat = materials.add(StandardMaterial::from(Color::srgb(0.18, 0.10, 0.22)));
    let speaker_mat = materials.add(StandardMaterial::from(Color::srgb(0.04, 0.04, 0.06)));
    let dj_z = -CLEARING_HALF + 60.0;
    // DJ booth — wide, shallow, waist-high. Solid: collides.
    spawn_collider_box(
        &mut commands,
        &mut meshes,
        &dj_mat,
        Vec3::new(0.0, 30.0, dj_z),
        Vec3::new(160.0, 60.0, 40.0),
    );
    // Speakers — tall narrow boxes flanking the DJ. Tagged with
    // `Speaker` so the audio system can attach `AudioPlayer` +
    // `PlaybackSettings::SPATIAL` to them in a later startup
    // system (see `audio::setup_audio`). Also colliders — you can't
    // walk through a speaker.
    for x in [-130.0_f32, 130.0] {
        let speaker_size = Vec3::new(50.0, 180.0, 50.0);
        let speaker_mesh = meshes.add(Cuboid::new(
            speaker_size.x,
            speaker_size.y,
            speaker_size.z,
        ));
        commands.spawn((
            Speaker,
            Mesh3d(speaker_mesh),
            MeshMaterial3d(speaker_mat.clone()),
            Transform::from_xyz(x, 90.0, dj_z),
            AabbCollider::cuboid(speaker_size),
        ));
    }

    // ----- Bar (west side of the clearing) -----
    let bar_mat = materials.add(StandardMaterial::from(Color::srgb(0.20, 0.13, 0.07)));
    spawn_collider_box(
        &mut commands,
        &mut meshes,
        &bar_mat,
        Vec3::new(-CLEARING_HALF + 40.0, 45.0, 0.0),
        Vec3::new(40.0, 90.0, 360.0),
    );

    // ----- Dancefloor square (centre, slightly raised, distinct color) -----
    //
    // Metallic + low-roughness so the coloured truss spots actually
    // bounce off it instead of the surface absorbing every photon.
    let dancefloor_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.18, 0.10, 0.22),
        metallic: 0.5,
        perceptual_roughness: 0.25,
        reflectance: 0.6,
        ..default()
    });
    let dancefloor_mesh = meshes.add(
        Plane3d::new(Vec3::Y, Vec2::splat(DANCEFLOOR_HALF))
            .mesh()
            .build(),
    );
    commands.spawn((
        Mesh3d(dancefloor_mesh),
        MeshMaterial3d(dancefloor_mat),
        Transform::from_xyz(0.0, 0.2, 0.0),
    ));

    // ----- Truss — horizontal beam above the dancefloor + 6 moving spots -----
    //
    // The truss itself is a thin Cuboid running east-west at the back
    // of the dancefloor. SpotLights hang below it pointing down, each
    // sweeping yaw + cycling hue at slightly different rates so the
    // dancefloor reads as continuously animated.
    let truss_mat = materials.add(StandardMaterial::from(Color::srgb(0.06, 0.06, 0.08)));
    let truss_y = 200.0;
    let truss_z = -60.0;
    spawn_box(
        &mut commands,
        &mut meshes,
        &truss_mat,
        Vec3::new(0.0, truss_y, truss_z),
        Vec3::new(560.0, 14.0, 14.0),
    );
    let truss_spot_count = 6;
    for i in 0..truss_spot_count {
        let i_f = i as f32;
        let lane = (i_f / (truss_spot_count as f32 - 1.0)) * 2.0 - 1.0; // [-1, 1]
        let x = lane * 240.0;
        let phase = i_f * 0.42;
        let yaw_freq = 0.45 + (i_f * 0.07);
        let color_period_s = 6.0 + (i_f * 1.3);
        let hue_offset = i_f * 60.0;
        commands.spawn((
            SpotLight {
                color: Color::srgb(1.0, 1.0, 1.0),
                intensity: 8_000_000.0,
                range: 800.0,
                outer_angle: 0.32,
                inner_angle: 0.16,
                shadow_maps_enabled: false,
                ..default()
            },
            // Up is world Y. The earlier Z was degenerate when the
            // forward direction was vertical and produced silent
            // near-zero cone rotations.
            Transform::from_xyz(x, truss_y - 8.0, truss_z)
                .looking_at(Vec3::new(x, 0.0, 0.0), Vec3::Y),
            TrussSpot {
                phase,
                yaw_freq,
                yaw_amp: 0.6,
                color_period_s,
                hue_offset,
            },
        ));
    }

    // ----- Strobes — four animated PointLights at dancefloor corners -----
    //
    // White only. Default mode is `StrobeMode::Off` so the dancefloor
    // is quiet until the user activates them (R24's click-cycle slice
    // adds the mouse interaction; until then they read as fixture-only
    // props that don't fire).
    let strobe_specs = [
        (DANCEFLOOR_HALF, DANCEFLOOR_HALF, 0.0, 3.1),
        (-DANCEFLOOR_HALF, DANCEFLOOR_HALF, 0.4, 2.7),
        (DANCEFLOOR_HALF, -DANCEFLOOR_HALF, 0.8, 3.5),
        (-DANCEFLOOR_HALF, -DANCEFLOOR_HALF, 1.2, 2.9),
    ];
    let strobe_fixture_mesh = meshes.add(Cuboid::new(18.0, 18.0, 18.0));
    for (x, z, phase, frequency) in strobe_specs {
        let fixture_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(0.05, 0.05, 0.05),
            emissive: LinearRgba::BLACK,
            ..default()
        });
        commands.spawn((
            Mesh3d(strobe_fixture_mesh.clone()),
            MeshMaterial3d(fixture_mat.clone()),
            PointLight {
                color: Color::WHITE,
                intensity: 0.0,
                range: 600.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(x, 90.0, z),
            Strobe {
                phase,
                frequency,
                mode: StrobeMode::Off,
                fixture_material: fixture_mat,
            },
        ));
    }

    // ----- Bar lights — magenta + blue PointLights along the bar -----
    //
    // The bar runs along x = -CLEARING_HALF + 40 with depth ±180 in z.
    // Two PointLights at ±90 z give the bar a magenta-and-blue glow
    // that doesn't bleed across the dancefloor.
    for (z, color) in [
        (-90.0_f32, Color::srgb(1.0, 0.15, 0.8)),
        (90.0, Color::srgb(0.2, 0.4, 1.0)),
    ] {
        commands.spawn((
            PointLight {
                color,
                intensity: 1_800_000.0,
                range: 280.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(-CLEARING_HALF + 70.0, 80.0, z),
        ));
    }

    // ----- Back lights — behind the DJ booth -----
    //
    // Two amber spots aimed back at the booth from behind so the DJ
    // silhouettes against the lit panels instead of disappearing into
    // the unlit forest beyond.
    let back_z = -CLEARING_HALF + 20.0;
    for x in [-120.0_f32, 120.0] {
        commands.spawn((
            PointLight {
                color: Color::srgb(1.0, 0.55, 0.15),
                intensity: 1_200_000.0,
                range: 240.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(x, 110.0, back_z),
        ));
    }
}

fn spawn_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mat: &Handle<StandardMaterial>,
    pos: Vec3,
    size: Vec3,
) {
    let mesh = meshes.add(Cuboid::new(size.x, size.y, size.z));
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(pos),
    ));
}

/// Same as `spawn_box` but also attaches an `AabbCollider` so the
/// physics system stops the player walking through it. Used for the
/// DJ booth and the bar — anything you can't pass through.
fn spawn_collider_box(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mat: &Handle<StandardMaterial>,
    pos: Vec3,
    size: Vec3,
) {
    let mesh = meshes.add(Cuboid::new(size.x, size.y, size.z));
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(pos),
        AabbCollider::cuboid(size),
    ));
}

/// Drives each Strobe's intensity from a sinusoid in time. Frequencies +
/// phases are deliberately mismatched so the four don't return to bright
/// at the same instant. Strobes in `StrobeMode::Off` stay dark — the
/// fixture's emissive + the PointLight's intensity both go to zero.
pub fn pulse_strobes(
    time: Res<Time>,
    mut lights: Query<(&mut PointLight, &Strobe)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let t = time.elapsed_secs();
    for (mut light, strobe) in &mut lights {
        let pulse = match strobe.mode {
            StrobeMode::Off => 0.0,
            StrobeMode::Strobing => {
                // Square wave-ish — clip the sin so the off phase is
                // fully dark and the on phase is a quick bright flash.
                let raw = ((t + strobe.phase) * strobe.frequency
                    * std::f32::consts::TAU)
                    .sin();
                if raw > 0.55 { 1.0 } else { 0.0 }
            }
        };
        light.intensity = pulse * 12_000_000.0;
        light.color = Color::WHITE;
        if let Some(mut mat) = materials.get_mut(&strobe.fixture_material) {
            mat.emissive = LinearRgba::WHITE * (pulse * 200.0);
        }
    }
}

/// Per-frame: each truss spotlight cycles its hue around the color
/// wheel and sweeps its yaw across the dancefloor. The two motions
/// run at different periods so a spot is rarely the same color at
/// the same aim point.
pub fn pulse_truss_lights(
    time: Res<Time>,
    mut spots: Query<(&mut SpotLight, &mut Transform, &TrussSpot)>,
) {
    let t = time.elapsed_secs();
    for (mut light, mut transform, spot) in &mut spots {
        // Yaw sweep — a sinusoid around the spot's mount position.
        let yaw = ((t + spot.phase) * spot.yaw_freq * std::f32::consts::TAU).sin()
            * spot.yaw_amp;
        // Aim downward at the floor offset by yaw radians around Y.
        let aim_x = transform.translation.x + yaw.sin() * 200.0;
        let aim_z = yaw.cos() * 200.0;
        transform.look_at(Vec3::new(aim_x, 0.0, aim_z), Vec3::Z);

        // Hue cycle — convert HSL to RGB by hand to avoid pulling
        // bevy_color::Hsla import baggage in this slice. Hue 0..360.
        let hue = ((t / spot.color_period_s) * 360.0 + spot.hue_offset) % 360.0;
        light.color = hsl_to_color(hue, 1.0, 0.5);
    }
}

fn hsl_to_color(hue_deg: f32, sat: f32, light: f32) -> Color {
    let c = (1.0 - (2.0 * light - 1.0).abs()) * sat;
    let h_prime = hue_deg / 60.0;
    let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_prime as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = light - c / 2.0;
    Color::srgb(r1 + m, g1 + m, b1 + m)
}
