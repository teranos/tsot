// Shared scene types + snapshot logic. Used by both the native
// wgpu render (render.rs) and the wasm hand-wired render
// (render_web.rs) so the WGSL and buffer layouts stay one source.

use bevy_app::App;
use bevy_math::Vec3;

use crate::campfire;
use crate::map::Pin;
use crate::physics::{self, AabbCollider, NpcMarker, PlayerMarker, Position};
use crate::trail::TrailMarker;
use crate::room;
use crate::trees;

pub const SHADER_WGSL: &str = r#"
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct IIn {
    @location(2) i_pos: vec3<f32>,
    @location(3) i_color: vec3<f32>,
    @location(4) i_scale: vec3<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs(v: VIn, i: IIn) -> VOut {
    let world = v.pos * i.i_scale + i.i_pos;
    var o: VOut;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    o.normal = normalize(v.normal);
    o.color = i.i_color;
    return o;
}

const LIGHT_DIR: vec3<f32> = vec3<f32>(0.3, 0.85, 0.4);
const AMBIENT: f32 = 0.25;

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    return vec4<f32>(in.color * k, 1.0);
}
"#;

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

    pub fn follow(player: [f32; 3], floor_half: f32) -> Self {
        let d = floor_half * 1.2;
        Self {
            eye: [player[0] + d, player[1] + d, player[2] + d],
            target: player,
            up: [0.0, 1.0, 0.0],
            half_extent: floor_half * 0.7,
            near: 100.0,
            far: floor_half * 6.0,
        }
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

/// One draw-call instance. Positions are world coords; scale is per-
/// axis stretch of the unit cube (-0.5..0.5); color is linear RGB.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SceneInstance {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub scale: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GpuVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

pub fn cube_geometry() -> Vec<GpuVertex> {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),   // +Z
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // -Z
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),  // +X
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),  // -X
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),  // +Y
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),  // -Y
    ];
    let mut out: Vec<GpuVertex> = Vec::with_capacity(36);
    for (n, u, v) in faces {
        let center = [n[0] * 0.5, n[1] * 0.5, n[2] * 0.5];
        let mk = |su: f32, sv: f32| GpuVertex {
            pos: [
                center[0] + u[0] * su * 0.5 + v[0] * sv * 0.5,
                center[1] + u[1] * su * 0.5 + v[1] * sv * 0.5,
                center[2] + u[2] * su * 0.5 + v[2] * sv * 0.5,
            ],
            normal: n,
        };
        let c00 = mk(-1.0, -1.0);
        let c10 = mk(1.0, -1.0);
        let c11 = mk(1.0, 1.0);
        let c01 = mk(-1.0, 1.0);
        out.push(c00);
        out.push(c10);
        out.push(c11);
        out.push(c00);
        out.push(c11);
        out.push(c01);
    }
    out
}

/// Reinterpret a `&[T]` where T is `#[repr(C)] Copy` as raw bytes.
pub fn as_bytes<T: Copy>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(slice.as_ptr() as *const u8, std::mem::size_of_val(slice))
    }
}

pub struct SceneSnapshot {
    pub trees: Vec<Vec3>,
    pub obstacles: Vec<Vec3>,
    pub fires: Vec<(Vec3, f32)>,
    pub npcs: Vec<Vec3>,
    pub pins: Vec<Vec3>,
    pub trails: Vec<Vec3>,
    pub player: Vec3,
}

pub fn snapshot_scene(app: &mut App) -> SceneSnapshot {
    let world = app.world_mut();
    let mut tree_q =
        world.query_filtered::<&Position, bevy_ecs::prelude::With<trees::TreeTrunk>>();
    let trees: Vec<Vec3> = tree_q.iter(world).map(|p| p.0).collect();
    let mut obs_q = world.query_filtered::<&Position, (
        bevy_ecs::prelude::With<AabbCollider>,
        bevy_ecs::prelude::Without<PlayerMarker>,
        bevy_ecs::prelude::Without<trees::TreeTrunk>,
        bevy_ecs::prelude::Without<campfire::Campfire>,
    )>();
    let obstacles: Vec<Vec3> = obs_q.iter(world).map(|p| p.0).collect();
    let mut fire_q = world.query::<(&Position, &campfire::Campfire)>();
    let fires: Vec<(Vec3, f32)> = fire_q
        .iter(world)
        .map(|(p, f)| (p.0, f.intensity))
        .collect();
    let mut player_q = world
        .query_filtered::<&Position, bevy_ecs::prelude::With<physics::PlayerMarker>>();
    let player = player_q
        .iter(world)
        .next()
        .map(|p| p.0)
        .unwrap_or(Vec3::ZERO);
    let mut npc_q = world.query_filtered::<&Position, bevy_ecs::prelude::With<NpcMarker>>();
    let npcs: Vec<Vec3> = npc_q.iter(world).map(|p| p.0).collect();
    let mut pin_q = world.query_filtered::<&Position, bevy_ecs::prelude::With<Pin>>();
    let pins: Vec<Vec3> = pin_q.iter(world).map(|p| p.0).collect();
    let mut trail_q = world.query_filtered::<&Position, bevy_ecs::prelude::With<TrailMarker>>();
    let trails: Vec<Vec3> = trail_q.iter(world).map(|p| p.0).collect();
    SceneSnapshot {
        trees,
        obstacles,
        fires,
        npcs,
        pins,
        trails,
        player,
    }
}

pub fn snapshot_to_instances(snap: &SceneSnapshot) -> Vec<SceneInstance> {
    let floor_half = room::FLOOR_HALF;
    let mut instances: Vec<SceneInstance> = Vec::with_capacity(
        1 + snap.trees.len() + snap.obstacles.len() + snap.fires.len() + snap.trails.len() + 1,
    );
    instances.push(SceneInstance {
        pos: [0.0, -50.0, 0.0],
        color: [0.09, 0.11, 0.15],
        scale: [floor_half * 2.0, 100.0, floor_half * 2.0],
    });
    // Trail — a thin flat rectangle sitting just above the ground.
    // Length is baked in via crate::trail::TRAIL_END_Z - TRAIL_START_Z.
    let trail_length = crate::trail::TRAIL_END_Z - crate::trail::TRAIL_START_Z;
    for t in &snap.trails {
        instances.push(SceneInstance {
            pos: [t.x, t.y, t.z],
            color: [0.18, 0.15, 0.10],
            scale: [crate::trail::TRAIL_WIDTH, 1.0, trail_length],
        });
    }
    for t in &snap.trees {
        instances.push(SceneInstance {
            pos: [t.x, 60.0, t.z],
            color: [0.13, 0.77, 0.37],
            scale: [40.0, 130.0, 40.0],
        });
    }
    for o in &snap.obstacles {
        instances.push(SceneInstance {
            pos: [o.x, 40.0, o.z],
            color: [0.92, 0.70, 0.03],
            scale: [60.0, 80.0, 60.0],
        });
    }
    for (fire_pos, intensity) in &snap.fires {
        let i = intensity.clamp(0.5, 1.5);
        instances.push(SceneInstance {
            pos: [fire_pos.x, 30.0, fire_pos.z],
            color: [1.0 * i, 0.45 * i, 0.08 * i],
            scale: [50.0, 60.0, 50.0],
        });
    }
    for n in &snap.npcs {
        instances.push(SceneInstance {
            pos: [n.x, 60.0, n.z],
            color: [0.95, 0.25, 0.20],
            scale: [70.0, 140.0, 70.0],
        });
    }
    // Named-zone markers — thin emissive-yellow rods so the layout
    // reads at a glance. Rave's rod+sphere+label overlay lands when
    // we grow text primitives in the shim.
    for p in &snap.pins {
        instances.push(SceneInstance {
            pos: [p.x, 40.0, p.z],
            color: [1.0, 0.9, 0.2],
            scale: [20.0, 80.0, 20.0],
        });
    }
    instances.push(SceneInstance {
        pos: [snap.player.x, 60.0, snap.player.z],
        color: [0.13, 0.83, 0.93],
        scale: [70.0, 140.0, 70.0],
    });
    instances
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_geometry_is_36_verts_on_unit_bounds_with_unit_normals() {
        let verts = cube_geometry();
        assert_eq!(verts.len(), 36);
        for v in &verts {
            for c in v.pos {
                assert!(c.abs() <= 0.5 + 1e-6, "vertex out of unit cube: {:?}", v.pos);
            }
            let n2: f32 = v.normal.iter().map(|c| c * c).sum();
            assert!((n2 - 1.0).abs() < 1e-4, "normal not unit: {:?}", v.normal);
        }
    }

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
