//! Room scaffold — the walkable area the rave happens in.
//!
//! Owns: floor plane, player marker, velocity, WASD/touch movement
//! clamped to the floor, third-person follow camera. No cells, no
//! algae, no water — those were the inherited universe prototype
//! and have been stripped. The room is a flat XZ square at Y=0;
//! the player stays floor-locked.

use bevy::prelude::*;

/// Floor extent — half-size of the playable XZ square at Y=0. The
/// player clamps to `[-FLOOR_HALF, FLOOR_HALF]` on X and Z.
pub const FLOOR_HALF: f32 = 500.0;

/// Identifies the local player entity. Networking code reads the
/// transform from the entity carrying this marker.
#[derive(Component)]
pub struct PlayerCell;

#[derive(Component)]
pub struct Velocity(pub Vec3);

/// Spawn the floor + player at room boot. Camera is owned by
/// `crate::setup` (the shared scene setup) so it can sit alongside
/// the lights without duplicate `Camera3d` entities.
pub fn setup_room(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Floor — a flat square at Y=0. Slightly darker than ClearColor
    // so the edge between floor and "void beyond" reads as a room.
    let floor_mesh = meshes.add(
        Plane3d::new(Vec3::Y, Vec2::splat(FLOOR_HALF))
            .mesh()
            .build(),
    );
    let floor_mat = materials.add(StandardMaterial::from(Color::srgb(0.07, 0.09, 0.13)));
    commands.spawn((
        Mesh3d(floor_mesh),
        MeshMaterial3d(floor_mat),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    // Player — a placeholder sphere standing on the floor. The
    // sphere's centre sits at PLAYER_RADIUS above Y=0 so it doesn't
    // intersect the plane.
    const PLAYER_RADIUS: f32 = 20.0;
    let player_mesh = meshes.add(Sphere::new(PLAYER_RADIUS));
    let player_mat = materials.add(StandardMaterial::from(Color::srgb(0.35, 0.85, 0.55)));
    commands.spawn((
        PlayerCell,
        Velocity(Vec3::ZERO),
        Mesh3d(player_mesh),
        MeshMaterial3d(player_mat),
        Transform::from_xyz(0.0, PLAYER_RADIUS, 0.0),
    ));
}

// Below this many screen pixels from the touch origin, the virtual
// joystick reads as centred — a resting thumb or a tap-meant-as-tap
// doesn't creep the player.
const TOUCH_DEADZONE_PX: f32 = 18.0;
// Drag distance at which the joystick is fully deflected. Past it the
// direction is unchanged and magnitude saturates at 1.
const TOUCH_JOY_RADIUS_PX: f32 = 90.0;

/// Touch-drag (screen pixels, y pointing down) → horizontal-plane move
/// direction. x maps to world +x (east); the screen-down axis maps to
/// world +z (toward the trailing camera) — the same axes `move_player`
/// drives from D and S. Pure (no ECS) so the deadzone/radius math is
/// unit-testable without the Bevy compile.
pub fn touch_drag_to_plane(dx: f32, dy: f32, deadzone: f32, radius: f32) -> Vec2 {
    let v = Vec2::new(dx, dy);
    let len = v.length();
    if len < deadzone {
        return Vec2::ZERO;
    }
    (v / len) * (len / radius).min(1.0)
}

/// WASD on the XZ plane, or — with no keyboard (mobile) — a touch-drag
/// virtual joystick driving the same plane. No Y axis: the player
/// stays floor-locked. Walls of the floor clamp position and zero the
/// perpendicular velocity component.
pub fn move_player(
    keys: Res<ButtonInput<KeyCode>>,
    touches: Res<Touches>,
    time: Res<Time>,
    mut players: Query<(&mut Transform, &mut Velocity), With<PlayerCell>>,
) {
    // While the chat input is focused, WASD belongs to the textbox,
    // not the player. Without this, typing "w" in chat also moves
    // the player — Bevy and the DOM input both receive the keystroke.
    if crate::chat::is_chat_focused() {
        return;
    }
    let mut accel = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        accel.z -= 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        accel.z += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        accel.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        accel.x += 1.0;
    }
    if let Some(touch) = touches.iter().next() {
        let start = touch.start_position();
        let pos = touch.position();
        let plane = touch_drag_to_plane(
            pos.x - start.x,
            pos.y - start.y,
            TOUCH_DEADZONE_PX,
            TOUCH_JOY_RADIUS_PX,
        );
        accel.x += plane.x;
        accel.z += plane.y;
    }
    let accel = accel.normalize_or_zero() * 900.0;
    let drag_per_sec = 2.4;
    let dt = time.delta_secs();
    for (mut t, mut v) in &mut players {
        v.0 += accel * dt;
        let drag = (1.0 - drag_per_sec * dt).max(0.0);
        v.0 *= drag;
        t.translation += v.0 * dt;
        // Floor walls — clamp XZ, leave Y at the spawn height.
        if t.translation.x.abs() > FLOOR_HALF {
            t.translation.x = t.translation.x.clamp(-FLOOR_HALF, FLOOR_HALF);
            v.0.x = 0.0;
        }
        if t.translation.z.abs() > FLOOR_HALF {
            t.translation.z = t.translation.z.clamp(-FLOOR_HALF, FLOOR_HALF);
            v.0.z = 0.0;
        }
    }
}

/// Third-person follow — camera trails behind and above the player at
/// roughly 50° pitch (top-down feel, but with enough horizontal angle
/// that lights + furniture stand out as 3D rather than reading flat).
pub fn camera_follow(
    players: Query<&Transform, (With<PlayerCell>, Without<Camera3d>)>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let Some(player_t) = players.iter().next() else {
        return;
    };
    let offset = Vec3::new(0.0, 300.0, 250.0);
    for mut cam_t in &mut cameras {
        cam_t.translation = player_t.translation + offset;
        cam_t.look_at(player_t.translation, Vec3::Y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_reads_as_centred() {
        assert_eq!(
            touch_drag_to_plane(0.0, 0.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX),
            Vec2::ZERO
        );
        // Just inside the deadzone is still no movement.
        assert_eq!(
            touch_drag_to_plane(10.0, 10.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX),
            Vec2::ZERO
        );
    }

    #[test]
    fn axes_map_screen_to_world_plane() {
        // Screen +x → world +x (east, the D key axis).
        let east = touch_drag_to_plane(80.0, 0.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX);
        assert!(east.x > 0.9 && east.y.abs() < 1e-3);
        // Screen +y (down) → world +z (toward camera, the S key axis).
        let south = touch_drag_to_plane(0.0, 80.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX);
        assert!(south.y > 0.9 && south.x.abs() < 1e-3);
    }

    #[test]
    fn magnitude_saturates_at_radius() {
        // A drag well past the radius clamps to unit length, keeping the
        // summed accel from over-driving normalize_or_zero downstream.
        let far = touch_drag_to_plane(500.0, 0.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX);
        assert!((far.length() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn partial_deflection_is_proportional() {
        // Half the radius → roughly half magnitude (direction preserved).
        let half = TOUCH_JOY_RADIUS_PX / 2.0;
        let v = touch_drag_to_plane(half, 0.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX);
        assert!((v.length() - 0.5).abs() < 1e-3);
    }
}
