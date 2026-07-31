use bevy_math::Vec3;

/// Top-down-with-tilt camera. Frustum spans the world so
/// [-FLOOR_HALF, +FLOOR_HALF] on XZ maps to the whole image.
pub struct SceneCamera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub half_extent: f32,
    pub near: f32,
    pub far: f32,
}

/// Follow-camera ortho half-extent, in world units — the visible
/// radius around the player. Absolute (not floor-relative) so growing
/// the world doesn't zoom the player's view back out. Smaller = more
/// zoomed in; the world runs past the screen edge, which is what makes
/// it read as open rather than as a board. (Future: minimap +
/// discovery to navigate the part you can't see.)
const FOLLOW_HALF_EXTENT: f32 = 1450.0;

/// Zoom bounds, in the same world units as `half_extent`. In is a
/// tighter frustum; out is a wider one. `FOLLOW_HALF_EXTENT` sits
/// inside this range and stays the resting value.
///
/// Bounded at both ends on purpose: past ZOOM_MIN a unit fills the
/// screen and you have lost the board, and past ZOOM_MAX the world
/// outruns chunk streaming and you are looking at what has not loaded.
pub const ZOOM_MIN: f32 = 400.0;
pub const ZOOM_MAX: f32 = 4000.0;

impl SceneCamera {
    pub fn default_for_floor(floor_half: f32) -> Self {
        // True isometric: equal offsets on all three axes → 45° yaw
        // around Y + arctan(1/√2) ≈ 35° elevation. Cubes project as
        // diamonds; world X and Z axes both draw at 45° to screen X.
        let d = floor_half * 1.2;
        Self {
            eye: [d, d, d],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            half_extent: floor_half * 1.4,
            near: 100.0,
            far: floor_half * 6.0,
        }
    }

    /// Camera looking at an arbitrary focus, at an arbitrary zoom.
    ///
    /// The isometric offset is the same one `follow` has always used —
    /// this only lifts the two things that were fixed (what it looks
    /// at, and how much it covers) into parameters, because an
    /// observer that can pan and zoom needs both, and an observer
    /// leashed to one entity at one magnification is not one.
    pub fn at(focus: [f32; 3], half_extent: f32, floor_half: f32) -> Self {
        let d = floor_half * 1.2;
        Self {
            eye: [focus[0] + d, focus[1] + d, focus[2] + d],
            target: focus,
            up: [0.0, 1.0, 0.0],
            half_extent,
            near: 100.0,
            far: floor_half * 6.0,
        }
    }

    pub fn follow(player: [f32; 3], floor_half: f32) -> Self {
        Self::at(player, FOLLOW_HALF_EXTENT, floor_half)
    }

    /// Project a world-space point to normalised clip coords in
    /// [-1, 1] × [-1, 1]. JS scales these to canvas pixels.
    pub fn world_to_clip(&self, world: [f32; 3]) -> [f32; 2] {
        let vp = self.view_proj();
        let cx = vp[0][0] * world[0] + vp[1][0] * world[1] + vp[2][0] * world[2] + vp[3][0];
        let cy = vp[0][1] * world[0] + vp[1][1] * world[1] + vp[2][1] * world[2] + vp[3][1];
        let cw = vp[0][3] * world[0] + vp[1][3] * world[1] + vp[2][3] * world[2] + vp[3][3];
        if cw.abs() < 1e-6 {
            [0.0, 0.0]
        } else {
            [cx / cw, cy / cw]
        }
    }

    /// view_proj packed row-major (column-major cols_array_2d).
    pub fn view_proj(&self) -> [[f32; 4]; 4] {
        let eye = Vec3::new(self.eye[0], self.eye[1], self.eye[2]);
        let target = Vec3::new(self.target[0], self.target[1], self.target[2]);
        let up = Vec3::new(self.up[0], self.up[1], self.up[2]);
        let view = bevy_math::Mat4::look_at_rh(eye, target, up);
        let proj = bevy_math::Mat4::orthographic_rh(
            -self.half_extent,
            self.half_extent,
            -self.half_extent,
            self.half_extent,
            self.near,
            self.far,
        );
        (proj * view).to_cols_array_2d()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_proj_maps_origin_into_clip_space() {
        let cam = SceneCamera::default_for_floor(3000.0);
        let vp = cam.view_proj();
        let col_major = |m: &[[f32; 4]; 4], r: usize| -> f32 {
            m[0][r] * 0.0 + m[1][r] * 0.0 + m[2][r] * 0.0 + m[3][r] * 1.0
        };
        let cx = col_major(&vp, 0);
        let cy = col_major(&vp, 1);
        let cz = col_major(&vp, 2);
        let cw = col_major(&vp, 3);
        assert!((cw - 1.0).abs() < 1e-4, "ortho w should be 1, got {cw}");
        assert!(cx.abs() < 1.0, "origin should be in-frustum x: {cx}");
        assert!(cy.abs() < 1.0, "origin should be in-frustum y: {cy}");
        assert!((0.0..=1.0).contains(&cz), "origin should be in-frustum z: {cz}");
    }

    #[test]
    fn follow_camera_maps_player_to_ndc_center() {
        let player = [1000.0, 20.0, -500.0];
        let cam = SceneCamera::follow(player, 3000.0);
        let vp = cam.view_proj();
        let col_major = |m: &[[f32; 4]; 4], r: usize| -> f32 {
            m[0][r] * player[0] + m[1][r] * player[1] + m[2][r] * player[2] + m[3][r] * 1.0
        };
        let cx = col_major(&vp, 0);
        let cy = col_major(&vp, 1);
        let cw = col_major(&vp, 3);
        assert!((cw - 1.0).abs() < 1e-4, "ortho w should be 1, got {cw}");
        assert!(cx.abs() < 1e-3, "player should be at NDC x=0, got {cx}");
        assert!(cy.abs() < 1e-3, "player should be at NDC y=0, got {cy}");
    }
}
