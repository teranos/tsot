// Shared scene types + snapshot logic. Used by both the native
// wgpu render (render.rs) and the wasm hand-wired render
// (render_web.rs) so the WGSL and buffer layouts stay one source.

use bevy_app::App;
use bevy_math::Vec3;

use crate::campfire;
use crate::jukebox::Jukebox;
use crate::map::Pin;
use crate::physics::{self, AabbCollider, NpcMarker, PlayerMarker, Position};
use crate::remote_players::{RemotePlayers, color_for_peer};
use crate::trail::TrailMarker;
use crate::template::{PropKind, StructureProp};
use crate::trees;

/// UI overlay shader — draws screen-space quads. Vertices computed
/// from vertex_index (6 per instance, two triangles). Instance data
/// is the quad center + half-size in NDC + color + alpha. Shares
/// the same bind group layout as the world pipeline for a
/// no-branch pipeline-layout reuse; the camera uniform is
/// declared here so the pipeline validates, but unused.
pub const UI_SHADER_WGSL: &str = r#"
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> _camera: Camera;

struct UiInstance {
    @location(0) center_ndc: vec2<f32>,
    @location(1) half_size_ndc: vec2<f32>,
    @location(2) color: vec3<f32>,
    @location(3) alpha: f32,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: UiInstance) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );
    let corner = corners[vi];
    let ndc = inst.center_ndc + corner * inst.half_size_ndc;
    var o: VOut;
    o.clip = vec4<f32>(ndc, 0.0, 1.0);
    o.color = vec4<f32>(inst.color, inst.alpha);
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// Ghost pass shader — cut-away walls + roof render here at low alpha
/// instead of vanishing, so the player still sees the outline of the
/// building they're inside. Same vertex + instance layout as the world
/// cube shader; distinct pipeline from glass so its alpha (and future
/// per-instance tuning) evolves separately.
pub const GHOST_SHADER_WGSL: &str = r#"
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
/// Deliberately faint — the ghost is a hint of the wall's outline,
/// not a wall you look through.
const GHOST_ALPHA: f32 = 0.15;

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    return vec4<f32>(in.color * k, GHOST_ALPHA);
}
"#;

/// Glass pass shader — identical geometry to the world cube shader, but
/// the fragment emits a low constant alpha so windows read as real
/// see-through glass. Drawn after the opaque world with alpha blending,
/// depth-tested against the world (so glass behind a wall is occluded)
/// but not depth-writing (so overlapping panes blend). Same vertex +
/// instance layout as SHADER_WGSL, so it reuses the cube geometry and
/// SceneInstance buffer.
pub const GLASS_SHADER_WGSL: &str = r#"
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
const GLASS_ALPHA: f32 = 0.34;

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    // Pre-multiply by alpha? No — the pipeline uses straight-alpha
    // blending (src-alpha, one-minus-src-alpha), so emit straight RGBA.
    return vec4<f32>(in.color * k, GLASS_ALPHA);
}
"#;

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

/// Follow-camera ortho half-extent, in world units — the visible
/// radius around the player. Absolute (not floor-relative) so growing
/// the world doesn't zoom the player's view back out. Smaller = more
/// zoomed in; the world runs past the screen edge, which is what makes
/// it read as open rather than as a board. (Future: minimap +
/// discovery to navigate the part you can't see.)
const FOLLOW_HALF_EXTENT: f32 = 1450.0;

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
            half_extent: FOLLOW_HALF_EXTENT,
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

pub struct RemotePeerDot {
    pub pos: Vec3,
    pub color: [f32; 3],
}

pub struct SceneSnapshot {
    pub trees: Vec<(Vec3, f32)>,
    pub obstacles: Vec<Vec3>,
    pub fires: Vec<(Vec3, f32)>,
    pub npcs: Vec<Vec3>,
    pub pins: Vec<Vec3>,
    pub trails: Vec<Vec3>,
    pub remote_peers: Vec<RemotePeerDot>,
    pub structures: Vec<(Vec3, PropKind, Option<[f32; 3]>)>,
    pub jukeboxes: Vec<Vec3>,
    pub player: Vec3,
}

pub fn snapshot_scene(app: &mut App) -> SceneSnapshot {
    let world = app.world_mut();
    let mut tree_q = world.query::<(&Position, &trees::TreeTrunk)>();
    let trees: Vec<(Vec3, f32)> = tree_q.iter(world).map(|(p, t)| (p.0, t.height)).collect();
    let mut obs_q = world.query_filtered::<&Position, (
        bevy_ecs::prelude::With<AabbCollider>,
        bevy_ecs::prelude::Without<PlayerMarker>,
        bevy_ecs::prelude::Without<trees::TreeTrunk>,
        bevy_ecs::prelude::Without<campfire::Campfire>,
        bevy_ecs::prelude::Without<StructureProp>,
        bevy_ecs::prelude::Without<Jukebox>,
    )>();
    let obstacles: Vec<Vec3> = obs_q.iter(world).map(|p| p.0).collect();
    let mut juke_q = world.query_filtered::<&Position, bevy_ecs::prelude::With<Jukebox>>();
    let jukeboxes: Vec<Vec3> = juke_q.iter(world).map(|p| p.0).collect();
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
    let remote_peers: Vec<RemotePeerDot> = world
        .get_resource::<RemotePlayers>()
        .map(|r| {
            r.0.iter()
                .map(|(peer, e)| RemotePeerDot {
                    pos: e.pos,
                    color: color_for_peer(peer),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut struct_q = world.query::<(&Position, &StructureProp)>();
    let structures: Vec<(Vec3, PropKind, Option<[f32; 3]>)> = struct_q
        .iter(world)
        .map(|(p, s)| (p.0, s.kind, s.color))
        .collect();
    SceneSnapshot {
        trees,
        obstacles,
        fires,
        npcs,
        pins,
        trails,
        remote_peers,
        structures,
        jukeboxes,
        player,
    }
}

/// Per-kind colour + cube size for static structure props. Chairs and
/// tables read as distinct wooden furniture. The campfire never
/// reaches here (it renders through its own flickering path); it has a
/// fallback only so the match is total.
fn prop_appearance(kind: PropKind) -> ([f32; 3], [f32; 3]) {
    match kind {
        PropKind::Chair => ([0.30, 0.20, 0.12], [28.0, 36.0, 28.0]),
        PropKind::Table => ([0.42, 0.28, 0.14], [64.0, 28.0, 64.0]),
        // Walls are one CDDA tile long, thin across the run. NS runs
        // along Z (thin in X); EW runs along X (thin in Z); the plain
        // Wall (corner/junction) fills the tile. Sizes match the
        // colliders in template.rs.
        PropKind::Wall => ([0.48, 0.47, 0.50], [80.0, 220.0, 80.0]),
        PropKind::WallNS => ([0.48, 0.47, 0.50], [24.0, 220.0, 80.0]),
        PropKind::WallEW => ([0.48, 0.47, 0.50], [80.0, 220.0, 24.0]),
        // Flat roof slab, sits at ROOF_HEIGHT (elevation comes from the
        // prop's y position, not this box).
        PropKind::Roof => ([0.33, 0.30, 0.34], [80.0, 20.0, 80.0]),
        PropKind::Furniture => ([0.34, 0.26, 0.20], [50.0, 70.0, 50.0]),
        PropKind::Campfire => ([1.0, 0.45, 0.08], [50.0, 60.0, 50.0]),
        // Glass panes fill their tile like the wall they sit in; the
        // translucency comes from the glass pass, not the colour. These
        // are only reached through `snapshot_to_glass_instances`.
        PropKind::Window => ([0.55, 0.70, 0.85], [80.0, 220.0, 80.0]),
        PropKind::WindowNS => ([0.55, 0.70, 0.85], [24.0, 220.0, 80.0]),
        PropKind::WindowEW => ([0.55, 0.70, 0.85], [80.0, 220.0, 24.0]),
    }
}

/// The floor is a single plane that follows the player — no world
/// edge. Half-extent large enough to exceed the view and the streamed
/// chunk region, so a uniform floor always fills the screen no matter
/// how far you roam. (A flat, untextured plane sliding under you is
/// invisible.)
const FLOOR_FOLLOW_HALF: f32 = 6000.0;

pub fn snapshot_to_instances(snap: &SceneSnapshot) -> Vec<SceneInstance> {
    let mut instances: Vec<SceneInstance> = Vec::with_capacity(
        1 + snap.trees.len() + snap.obstacles.len() + snap.fires.len() + snap.trails.len() + 1,
    );
    instances.push(SceneInstance {
        pos: [snap.player.x, -50.0, snap.player.z],
        color: [0.09, 0.11, 0.15],
        scale: [FLOOR_FOLLOW_HALF * 2.0, 100.0, FLOOR_FOLLOW_HALF * 2.0],
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
    for (t, h) in &snap.trees {
        // A brown trunk under a green canopy, sized by the tree's own
        // height (varied, all taller than the 220 buildings).
        let trunk_h = h * 0.40;
        let canopy_h = h * 0.66;
        instances.push(SceneInstance {
            pos: [t.x, trunk_h * 0.5, t.z],
            color: [0.30, 0.20, 0.11],
            scale: [h * 0.06, trunk_h, h * 0.06],
        });
        instances.push(SceneInstance {
            pos: [t.x, trunk_h + canopy_h * 0.5 - h * 0.06, t.z],
            color: [0.13, 0.77, 0.37],
            scale: [h * 0.22, canopy_h, h * 0.22],
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
    // Remote peers — per-peer colour so two players are visually
    // distinct from each other and from NPCs.
    for p in &snap.remote_peers {
        instances.push(SceneInstance {
            pos: [p.pos.x, 60.0, p.pos.z],
            color: p.color,
            scale: [70.0, 140.0, 70.0],
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
    // Roof cut-away: when the player is genuinely under a roof, hide
    // that building's roof so the interior is visible. "Inside" = the
    // player is within a roof slab's footprint (half-width CDDA_TILE/2).
    // The cut-away is anchored to the FOUND overhead roof cell, not the
    // player, so a neighbouring building's roof/walls stay solid even
    // when they'd be within the player's own cut radius.
    let roof_half = crate::cdda::CDDA_TILE / 2.0;
    let overhead = snap.structures.iter().find(|(p, k, _)| {
        *k == PropKind::Roof
            && (p.x - snap.player.x).abs() <= roof_half
            && (p.z - snap.player.z).abs() <= roof_half
    });
    let under_roof = overhead.is_some();
    // Tight enough that a neighbouring building's walls stay solid
    // (buildings sit ≥ 1 chunk apart, so anything past 800 is a
    // different structure), loose enough to reach a big CDDA house's
    // far perimeter from the overhead cell.
    let cutaway_radius = 800.0_f32;
    let overhead_pos = overhead.map(|(p, _, _)| *p).unwrap_or(snap.player);
    // The isometric camera looks from +x,+z, so nearer props have a
    // larger x+z. When inside, hide this building's roof AND its
    // camera-facing walls (in front of the player) so the interior
    // shows; the far walls stay as a backdrop.
    let player_depth = snap.player.x + snap.player.z;

    // Structure props (walls, furniture, roof). Size comes from the
    // kind; colour is the prop's own tint (walls by material) or the
    // kind default. Ground props sit on the floor (base at y=0);
    // elevated props (the roof) carry their height in pos.y.
    for (pos, kind, tint) in &snap.structures {
        // Glass panes are drawn in the separate alpha-blended pass
        // (see `snapshot_to_glass_instances`), never here.
        if kind.is_window() {
            continue;
        }
        let in_footprint = (pos.x - overhead_pos.x).abs() < cutaway_radius
            && (pos.z - overhead_pos.z).abs() < cutaway_radius;
        if under_roof && in_footprint {
            if *kind == PropKind::Roof {
                continue; // see-through roof
            }
            let is_wall =
                matches!(kind, PropKind::Wall | PropKind::WallNS | PropKind::WallEW);
            if is_wall && (pos.x + pos.z) > player_depth + 40.0 {
                continue; // see-through camera-facing wall
            }
        }
        let (default_color, scale) = prop_appearance(*kind);
        let color = tint.unwrap_or(default_color);
        instances.push(SceneInstance {
            pos: [pos.x, pos.y + scale[1] * 0.5, pos.z],
            color,
            scale,
        });
    }
    // Purple jukebox — a squat solid box you toggle the music at.
    for j in &snap.jukeboxes {
        instances.push(SceneInstance {
            pos: [j.x, crate::jukebox::JUKEBOX_SIZE.y * 0.5, j.z],
            color: crate::jukebox::JUKEBOX_COLOR,
            scale: [
                crate::jukebox::JUKEBOX_SIZE.x,
                crate::jukebox::JUKEBOX_SIZE.y,
                crate::jukebox::JUKEBOX_SIZE.z,
            ],
        });
    }
    instances.push(SceneInstance {
        pos: [snap.player.x, 60.0, snap.player.z],
        color: [0.13, 0.83, 0.93],
        scale: [70.0, 140.0, 70.0],
    });
    instances
}

/// Every wall/roof cut away by `snapshot_to_instances` — surfaced here
/// as low-alpha ghost instances so the player still sees an outline of
/// the building they're inside, rather than the geometry silently
/// disappearing. Same eligibility test as the cut-away in
/// `snapshot_to_instances`, inverted: this returns exactly what the
/// opaque pass skips (never the other way around, so no double-draw).
pub fn snapshot_to_ghost_instances(snap: &SceneSnapshot) -> Vec<SceneInstance> {
    let roof_half = crate::cdda::CDDA_TILE / 2.0;
    let overhead = snap.structures.iter().find(|(p, k, _)| {
        *k == PropKind::Roof
            && (p.x - snap.player.x).abs() <= roof_half
            && (p.z - snap.player.z).abs() <= roof_half
    });
    let Some(overhead) = overhead else {
        return Vec::new();
    };
    let overhead_pos = overhead.0;
    let cutaway_radius = 800.0_f32;
    let player_depth = snap.player.x + snap.player.z;
    let mut out = Vec::new();
    for (pos, kind, tint) in &snap.structures {
        if kind.is_window() {
            continue;
        }
        let in_footprint = (pos.x - overhead_pos.x).abs() < cutaway_radius
            && (pos.z - overhead_pos.z).abs() < cutaway_radius;
        if !in_footprint {
            continue;
        }
        let is_ghost = *kind == PropKind::Roof
            || (matches!(
                kind,
                PropKind::Wall | PropKind::WallNS | PropKind::WallEW
            ) && (pos.x + pos.z) > player_depth + 40.0);
        if !is_ghost {
            continue;
        }
        let (default_color, scale) = prop_appearance(*kind);
        let color = tint.unwrap_or(default_color);
        out.push(SceneInstance {
            pos: [pos.x, pos.y + scale[1] * 0.5, pos.z],
            color,
            scale,
        });
    }
    out
}

/// The translucent glass panes, as instances for the alpha-blended
/// glass pass. Separate from `snapshot_to_instances` so the opaque
/// world can draw + write depth first, then glass blends over it. Glass
/// is see-through by construction, so — unlike opaque near walls — it's
/// never cut away when the player is inside.
pub fn snapshot_to_glass_instances(snap: &SceneSnapshot) -> Vec<SceneInstance> {
    let mut out = Vec::new();
    for (pos, kind, tint) in &snap.structures {
        if !kind.is_window() {
            continue;
        }
        let (default_color, scale) = prop_appearance(*kind);
        let color = tint.unwrap_or(default_color);
        out.push(SceneInstance {
            pos: [pos.x, pos.y + scale[1] * 0.5, pos.z],
            color,
            scale,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_follows_the_player_so_there_is_no_world_edge() {
        let snap = SceneSnapshot {
            trees: vec![],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![],
            jukeboxes: vec![],
            player: Vec3::new(123_456.0, 20.0, -98_765.0),
        };
        let instances = snapshot_to_instances(&snap);
        // The floor is the first instance; it sits under the player.
        assert_eq!(instances[0].pos[0], 123_456.0);
        assert_eq!(instances[0].pos[2], -98_765.0);
    }

    #[test]
    fn roof_is_seethrough_when_the_player_is_under_it() {
        let snap = |px: f32| SceneSnapshot {
            trees: vec![],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![(Vec3::new(0.0, 220.0, 0.0), PropKind::Roof, None)],
            jukeboxes: vec![],
            player: Vec3::new(px, 20.0, 0.0),
        };
        // The roof is the only instance above y=100; check its presence.
        let has_roof = |px: f32| snapshot_to_instances(&snap(px)).iter().any(|i| i.pos[1] > 100.0);
        assert!(!has_roof(0.0), "roof should be hidden when the player is under it");
        assert!(has_roof(5000.0), "roof should render when the player is far away");
        // Just outside the slab's half-width (CDDA_TILE/2 = 40): the
        // player is next to the wall but NOT under the roof, so the
        // cut-away must NOT trigger — walls/roof stay solid.
        assert!(
            has_roof(60.0),
            "standing close to a wall from outside must not make it see-through"
        );
    }

    fn structure_at(x: f32, y: f32, z: f32, kind: PropKind) -> (Vec3, PropKind, Option<[f32; 3]>) {
        (Vec3::new(x, y, z), kind, None)
    }

    #[test]
    fn cut_away_stays_within_the_building_you_are_actually_under() {
        // Player is inside building A (roof at origin). Building B sits
        // ~900 units east — well within the old 1100 blast radius but a
        // separate building (its own roof). B's camera-facing wall must
        // stay solid.
        let snap = SceneSnapshot {
            trees: vec![],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![
                // Building A: roof over the player.
                structure_at(0.0, 220.0, 0.0, PropKind::Roof),
                // Building B: its own roof 900 units east, plus a
                // camera-facing wall next to that roof.
                structure_at(900.0, 220.0, 900.0, PropKind::Roof),
                structure_at(900.0, 0.0, 900.0, PropKind::Wall),
            ],
            jukeboxes: vec![],
            player: Vec3::ZERO,
        };
        let opaque = snapshot_to_instances(&snap);
        let has_neighbor_wall = opaque.iter().any(|i| {
            i.pos[0] > 800.0 && (90.0..140.0).contains(&i.pos[1])
        });
        assert!(
            has_neighbor_wall,
            "neighbouring building's wall got cut — cut-away leaked past the current building's footprint: {:?}",
            opaque.iter().map(|i| (i.pos, i.scale)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn near_walls_are_cut_away_when_inside() {
        let snap = SceneSnapshot {
            trees: vec![],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![
                (Vec3::new(0.0, 220.0, 0.0), PropKind::Roof, None), // overhead → inside
                (Vec3::new(300.0, 0.0, 300.0), PropKind::Wall, None), // camera-facing (x+z>0)
                (Vec3::new(-300.0, 0.0, -300.0), PropKind::Wall, None), // far side
            ],
            jukeboxes: vec![],
            player: Vec3::ZERO,
        };
        let inst = snapshot_to_instances(&snap);
        // Walls render around y=110; only the far wall should survive.
        let walls: Vec<_> = inst.iter().filter(|i| (90.0..140.0).contains(&i.pos[1])).collect();
        assert_eq!(walls.len(), 1, "only the far wall should remain when inside");
        assert!(walls[0].pos[0] < 0.0, "the surviving wall is the far one");
    }

    #[test]
    fn cut_away_wall_and_roof_surface_as_ghost_instances() {
        let snap = SceneSnapshot {
            trees: vec![],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![
                (Vec3::new(0.0, 220.0, 0.0), PropKind::Roof, None), // overhead → inside
                (Vec3::new(300.0, 0.0, 300.0), PropKind::Wall, None), // camera-facing (x+z>0)
                (Vec3::new(-300.0, 0.0, -300.0), PropKind::Wall, None), // far side (still opaque)
            ],
            jukeboxes: vec![],
            player: Vec3::ZERO,
        };
        let opaque = snapshot_to_instances(&snap);
        let ghost = snapshot_to_ghost_instances(&snap);
        // The opaque pass keeps the far wall only.
        let opaque_walls: Vec<_> = opaque.iter().filter(|i| (90.0..140.0).contains(&i.pos[1])).collect();
        assert_eq!(opaque_walls.len(), 1);
        assert!(opaque_walls[0].pos[0] < 0.0);
        // Ghost carries what the opaque pass dropped: the camera-facing
        // wall + the roof — never the far wall (still opaque).
        assert_eq!(ghost.len(), 2, "expected roof + camera-facing wall as ghosts");
        assert!(ghost.iter().any(|i| i.pos[1] > 200.0), "ghost includes the roof");
        assert!(
            ghost.iter().any(|i| i.pos[0] > 0.0 && (90.0..140.0).contains(&i.pos[1])),
            "ghost includes the camera-facing wall"
        );
    }

    #[test]
    fn ghost_is_empty_when_not_under_roof() {
        let snap = SceneSnapshot {
            trees: vec![],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![
                (Vec3::new(0.0, 220.0, 0.0), PropKind::Roof, None),
                (Vec3::new(300.0, 0.0, 300.0), PropKind::Wall, None),
            ],
            jukeboxes: vec![],
            player: Vec3::new(5000.0, 0.0, 5000.0), // far from any roof
        };
        assert!(snapshot_to_ghost_instances(&snap).is_empty());
    }

    #[test]
    fn windows_render_in_the_glass_pass_not_the_opaque_pass() {
        let snap = SceneSnapshot {
            trees: vec![],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![
                (Vec3::new(300.0, 0.0, 0.0), PropKind::WallNS, None),
                (Vec3::new(300.0, 0.0, 80.0), PropKind::WindowNS, Some([0.5, 0.68, 0.82])),
            ],
            jukeboxes: vec![],
            player: Vec3::ZERO,
        };
        let opaque = snapshot_to_instances(&snap);
        let glass = snapshot_to_glass_instances(&snap);
        // The glass pass carries exactly the one window pane.
        assert_eq!(glass.len(), 1, "the window belongs to the glass pass");
        assert_eq!(glass[0].color, [0.5, 0.68, 0.82], "keeps its glass tint");
        // The opaque pass has the wall but not the window: no opaque
        // instance sits at the window's position.
        assert!(
            !opaque.iter().any(|i| i.pos[0] == 300.0 && i.pos[2] == 80.0),
            "the window must not be drawn opaque"
        );
        assert!(
            opaque.iter().any(|i| i.pos[0] == 300.0 && i.pos[2] == 0.0),
            "the wall still draws opaque"
        );
    }

    #[test]
    fn jukebox_renders_as_a_purple_box() {
        let snap = SceneSnapshot {
            trees: vec![],
            obstacles: vec![],
            fires: vec![],
            npcs: vec![],
            pins: vec![],
            trails: vec![],
            remote_peers: vec![],
            structures: vec![],
            jukeboxes: vec![Vec3::new(200.0, 0.0, 2400.0)],
            player: Vec3::ZERO,
        };
        let inst = snapshot_to_instances(&snap);
        assert!(
            inst.iter()
                .any(|i| i.color == crate::jukebox::JUKEBOX_COLOR && i.pos[0] == 200.0),
            "the jukebox should render at its position in its purple colour"
        );
    }

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
