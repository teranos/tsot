use bevy_math::{Vec2, Vec3};

/// Half-extent of the *content* region — the square around the origin
/// where whole-world placers (campsites) still generate. No longer a
/// wall: the floor and forest stream infinitely (see chunk.rs), so the
/// player can walk past this into endless forest. Only bounds the
/// not-yet-chunked content.
pub const FLOOR_HALF: f32 = 8000.0;

/// Where the player spawns — well south of the clearing. The trail
/// runs north from here to the clearing edge.
pub const SPAWN_POS: Vec3 = Vec3::new(0.0, 20.0, 2400.0);

/// Below this many screen pixels from the touch origin, the virtual
/// joystick reads as centred — a resting thumb or a tap-meant-as-tap
/// doesn't creep the player.
pub const TOUCH_DEADZONE_PX: f32 = 18.0;
/// Drag distance at which the joystick is fully deflected.
pub const TOUCH_JOY_RADIUS_PX: f32 = 90.0;

/// Touch-drag (screen pixels, y pointing down) → horizontal-plane
/// move direction. x maps to world +x (east); screen-down maps to
/// world +z (toward the trailing camera). Pure (no ECS) so the
/// deadzone/radius math is unit-testable without the Bevy compile.
pub fn touch_drag_to_plane(dx: f32, dy: f32, deadzone: f32, radius: f32) -> Vec2 {
    let v = Vec2::new(dx, dy);
    let len = v.length();
    if len < deadzone {
        return Vec2::ZERO;
    }
    (v / len) * (len / radius).min(1.0)
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
        assert_eq!(
            touch_drag_to_plane(10.0, 10.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX),
            Vec2::ZERO
        );
    }

    #[test]
    fn axes_map_screen_to_world_plane() {
        // Use a drag distance past TOUCH_JOY_RADIUS_PX so magnitude
        // saturates at 1.0 — makes the axis-mapping assertion crisp
        // instead of depending on the exact partial-deflection scale.
        let east = touch_drag_to_plane(100.0, 0.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX);
        assert!(east.x > 0.99 && east.y.abs() < 1e-3);
        let south = touch_drag_to_plane(0.0, 100.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX);
        assert!(south.y > 0.99 && south.x.abs() < 1e-3);
    }

    #[test]
    fn magnitude_saturates_at_radius() {
        let far = touch_drag_to_plane(500.0, 0.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX);
        assert!((far.length() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn partial_deflection_is_proportional() {
        let half = TOUCH_JOY_RADIUS_PX / 2.0;
        let v = touch_drag_to_plane(half, 0.0, TOUCH_DEADZONE_PX, TOUCH_JOY_RADIUS_PX);
        assert!((v.length() - 0.5).abs() < 1e-3);
    }
}
