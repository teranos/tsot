//! Floor plan — DJ booth, speakers, bar, toilets, entrance + garderobe,
//! dancefloor, strobes. All spawned at Startup; strobes are animated
//! every frame by `pulse_strobes` so they flash asynchronously.
//!
//! Coordinate system (camera looks roughly down from +Z, +Y):
//!   north (back wall) = -Z       south (entrance) = +Z
//!   west  (bar)       = -X       east  (toilets)  = +X
//!
//! All meshes are simple primitives (Cuboid + Plane3d). The PolyPizza
//! humanoid models are a separate slice — they'll replace the player +
//! peer spheres without touching this room scaffold.

use bevy::prelude::*;

use crate::room::FLOOR_HALF;

const WALL_HEIGHT: f32 = 80.0;
const WALL_THICKNESS: f32 = 8.0;
const DANCEFLOOR_HALF: f32 = 160.0;

/// Marker on each strobe PointLight so the animation system can find
/// them without grabbing every PointLight in the world.
#[derive(Component)]
pub struct Strobe {
    /// Phase offset in seconds — different per strobe so the four
    /// don't pulse in lockstep.
    pub phase: f32,
    /// Pulse frequency in Hz.
    pub frequency: f32,
    /// Base hue at full intensity.
    pub color: Color,
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

pub fn setup_floor_plan(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // ----- Perimeter walls -----
    //
    // Four low walls clip the floor visually. The south wall has a
    // gap in the middle for the entrance — built as two segments.
    let wall_mat = materials.add(StandardMaterial::from(Color::srgb(0.12, 0.13, 0.18)));

    // North (full width)
    spawn_wall(
        &mut commands,
        &mut meshes,
        &wall_mat,
        Vec3::new(0.0, WALL_HEIGHT / 2.0, -FLOOR_HALF),
        Vec3::new(FLOOR_HALF * 2.0, WALL_HEIGHT, WALL_THICKNESS),
    );
    // East (full depth)
    spawn_wall(
        &mut commands,
        &mut meshes,
        &wall_mat,
        Vec3::new(FLOOR_HALF, WALL_HEIGHT / 2.0, 0.0),
        Vec3::new(WALL_THICKNESS, WALL_HEIGHT, FLOOR_HALF * 2.0),
    );
    // West (full depth)
    spawn_wall(
        &mut commands,
        &mut meshes,
        &wall_mat,
        Vec3::new(-FLOOR_HALF, WALL_HEIGHT / 2.0, 0.0),
        Vec3::new(WALL_THICKNESS, WALL_HEIGHT, FLOOR_HALF * 2.0),
    );
    // South — two segments, leaving an 180-wide entrance in the middle.
    let south_gap_half = 90.0;
    let south_seg_len = FLOOR_HALF - south_gap_half;
    spawn_wall(
        &mut commands,
        &mut meshes,
        &wall_mat,
        Vec3::new(-(south_gap_half + south_seg_len / 2.0), WALL_HEIGHT / 2.0, FLOOR_HALF),
        Vec3::new(south_seg_len, WALL_HEIGHT, WALL_THICKNESS),
    );
    spawn_wall(
        &mut commands,
        &mut meshes,
        &wall_mat,
        Vec3::new(south_gap_half + south_seg_len / 2.0, WALL_HEIGHT / 2.0, FLOOR_HALF),
        Vec3::new(south_seg_len, WALL_HEIGHT, WALL_THICKNESS),
    );

    // ----- DJ booth + speakers (north wall) -----
    let dj_mat = materials.add(StandardMaterial::from(Color::srgb(0.18, 0.10, 0.22)));
    let speaker_mat = materials.add(StandardMaterial::from(Color::srgb(0.04, 0.04, 0.06)));
    let dj_z = -FLOOR_HALF + 60.0;
    // DJ booth — wide, shallow, waist-high.
    spawn_box(
        &mut commands,
        &mut meshes,
        &dj_mat,
        Vec3::new(0.0, 30.0, dj_z),
        Vec3::new(160.0, 60.0, 40.0),
    );
    // Speakers — tall narrow boxes flanking the DJ.
    for x in [-130.0_f32, 130.0] {
        spawn_box(
            &mut commands,
            &mut meshes,
            &speaker_mat,
            Vec3::new(x, 90.0, dj_z),
            Vec3::new(50.0, 180.0, 50.0),
        );
    }

    // ----- Bar (west wall) -----
    let bar_mat = materials.add(StandardMaterial::from(Color::srgb(0.20, 0.13, 0.07)));
    spawn_box(
        &mut commands,
        &mut meshes,
        &bar_mat,
        Vec3::new(-FLOOR_HALF + 40.0, 45.0, 0.0),
        Vec3::new(40.0, 90.0, 360.0),
    );

    // ----- Toilets (east wall, two stalls separated by a partition) -----
    let toilet_mat = materials.add(StandardMaterial::from(Color::srgb(0.10, 0.18, 0.20)));
    for z in [-90.0_f32, 90.0] {
        spawn_box(
            &mut commands,
            &mut meshes,
            &toilet_mat,
            Vec3::new(FLOOR_HALF - 40.0, 35.0, z),
            Vec3::new(70.0, 70.0, 100.0),
        );
    }

    // ----- Garderobe (south, just inside the entrance gap) -----
    let garderobe_mat = materials.add(StandardMaterial::from(Color::srgb(0.18, 0.16, 0.10)));
    spawn_box(
        &mut commands,
        &mut meshes,
        &garderobe_mat,
        Vec3::new(0.0, 35.0, FLOOR_HALF - 60.0),
        Vec3::new(120.0, 70.0, 30.0),
    );

    // ----- Dancefloor square (in the middle, slightly raised, distinct color) -----
    let dancefloor_mat = materials.add(StandardMaterial::from(Color::srgb(0.18, 0.10, 0.22)));
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
                intensity: 1_500_000.0,
                range: 600.0,
                outer_angle: 0.35,
                inner_angle: 0.18,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_xyz(x, truss_y - 8.0, truss_z)
                .looking_at(Vec3::new(x, 0.0, 0.0), Vec3::Z),
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
    let strobe_specs = [
        (DANCEFLOOR_HALF, DANCEFLOOR_HALF, Color::srgb(1.0, 0.2, 0.8), 0.0, 3.1),
        (-DANCEFLOOR_HALF, DANCEFLOOR_HALF, Color::srgb(0.2, 0.9, 1.0), 0.4, 2.7),
        (DANCEFLOOR_HALF, -DANCEFLOOR_HALF, Color::srgb(1.0, 0.85, 0.2), 0.8, 3.5),
        (-DANCEFLOOR_HALF, -DANCEFLOOR_HALF, Color::srgb(0.5, 0.2, 1.0), 1.2, 2.9),
    ];
    for (x, z, color, phase, frequency) in strobe_specs {
        commands.spawn((
            PointLight {
                color,
                intensity: 0.0,
                range: 350.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(x, 60.0, z),
            Strobe {
                phase,
                frequency,
                color,
            },
        ));
    }
}

fn spawn_wall(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mat: &Handle<StandardMaterial>,
    pos: Vec3,
    size: Vec3,
) {
    spawn_box(commands, meshes, mat, pos, size);
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

/// Drives each Strobe's intensity from a sinusoid in time. Frequencies +
/// phases are deliberately mismatched so the four don't return to bright
/// at the same instant — the dancefloor reads as alive, not metronome.
pub fn pulse_strobes(
    time: Res<Time>,
    mut lights: Query<(&mut PointLight, &Strobe)>,
) {
    let t = time.elapsed_secs();
    for (mut light, strobe) in &mut lights {
        // Square wave-ish — clip the sin so the off phase is fully dark
        // and the on phase is a quick bright flash, not a slow sweep.
        let raw = ((t + strobe.phase) * strobe.frequency * std::f32::consts::TAU).sin();
        let pulse = if raw > 0.55 { 1.0 } else { 0.0 };
        light.intensity = pulse * 2_500_000.0;
        light.color = strobe.color;
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
