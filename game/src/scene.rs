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

/// Instance for the MESH pipeline. Same as `SceneInstance` plus `axis`:
/// `xyz` is the unit direction the shader rotates the geometry's local
/// +Y onto (one baked cone → a limb pointing anywhere), and `w` is the
/// per-instance WIND SWAY weight (0 = rigid trunk, →1 = a thin twig that
/// flutters most). 52 bytes, `#[repr(C)]`: layout must match the mesh
/// WGSL's IIn (loc 3/4/5/6 at offsets 0/12/24/36) and the vertex-buffer
/// layouts on both render paths — all held to `INSTANCE_ATTRS`. A
/// vertical trunk sets `axis = [0,1,0,0]` (identity rotation, no sway).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MeshInstance {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub scale: [f32; 3],
    pub axis: [f32; 4],
}

/// One per-instance vertex attribute of `MeshInstance`.
pub struct InstanceAttr {
    /// `@location` in the WGSL and `shaderLocation` in the JS shim.
    pub location: u32,
    /// Byte offset into `MeshInstance`.
    pub offset: u64,
    /// WebGPU vertex-format name, exactly as the JS shim spells it.
    pub format: &'static str,
}

/// THE single source of truth for the `MeshInstance` vertex layout. The
/// same 48-byte record is described in four places — the `#[repr(C)]`
/// struct above (the PRODUCER of the bytes), the WGSL `@location` list,
/// the native `wgpu` attribute array (`render.rs` builds its array FROM
/// this const), and the hand-written JS shim (`web/src/main.ts`). This
/// const is what the other three must agree with; `render.rs` derives
/// from it, and `web_shim_mesh_instance_layout_matches_this_const` holds
/// the JS copy to it so the one hand-maintained descriptor can't drift
/// silently (only a real browser exercises the JS path, so nothing else
/// would catch it). No proto / codegen — the JS stays hand-inspectable,
/// a test just checks it.
pub const INSTANCE_ATTRS: &[InstanceAttr] = &[
    InstanceAttr { location: 3, offset: 0, format: "float32x3" },
    InstanceAttr { location: 4, offset: 12, format: "float32x3" },
    InstanceAttr { location: 5, offset: 24, format: "float32x3" },
    // axis is a vec4: xyz = orientation, w = wind sway weight.
    InstanceAttr { location: 6, offset: 36, format: "float32x4" },
];

/// Stride of the per-instance buffer — the size of one `MeshInstance`.
/// Passed to both render paths (`instanceStride` in JS), so it can't
/// disagree; the offsets are what need guarding.
pub const INSTANCE_STRIDE: u64 = std::mem::size_of::<MeshInstance>() as u64;

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

/// One structure prop in the snapshot: position, kind, colour override,
/// size override. Named to keep the SceneSnapshot type readable and
/// clippy quiet.
pub type StructureSnap = (Vec3, PropKind, Option<[f32; 3]>, Option<Vec3>);

pub struct SceneSnapshot {
    pub trees: Vec<(
        Vec3,
        f32,
        &'static crate::tree_mesh::TreeSpecies,
        crate::trees::LifeStage,
    )>,
    pub obstacles: Vec<Vec3>,
    pub fires: Vec<(Vec3, f32)>,
    pub npcs: Vec<Vec3>,
    pub pins: Vec<Vec3>,
    pub trails: Vec<Vec3>,
    pub remote_peers: Vec<RemotePeerDot>,
    pub structures: Vec<StructureSnap>,
    pub jukeboxes: Vec<Vec3>,
    pub player: Vec3,
}

pub fn snapshot_scene(app: &mut App) -> SceneSnapshot {
    let world = app.world_mut();
    let mut tree_q = world.query::<(&Position, &trees::TreeTrunk)>();
    let trees: Vec<(
        Vec3,
        f32,
        &'static crate::tree_mesh::TreeSpecies,
        crate::trees::LifeStage,
    )> = tree_q
        .iter(world)
        .map(|(p, t)| (p.0, t.height, t.species, t.stage))
        .collect();
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
    let structures: Vec<StructureSnap> = struct_q
        .iter(world)
        .map(|(p, s)| (p.0, s.kind, s.color, s.size))
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
        // Fence — bottom rail (single instance from prop_appearance).
        // The top rail is added by the structure loop as a second
        // instance, so the fence reads as two stacked thin bars with a
        // see-through gap between them (real-fence silhouette).
        PropKind::Fence => (FENCE_COLOR, [8.0, 6.0, 8.0]),
        PropKind::FenceNS => (FENCE_COLOR, [8.0, 6.0, 80.0]),
        PropKind::FenceEW => (FENCE_COLOR, [80.0, 6.0, 8.0]),
        // Toilet — white ceramic block, roughly toilet-sized.
        PropKind::Toilet => ([0.92, 0.94, 0.94], [36.0, 50.0, 44.0]),
    }
}

/// Weathered wood — same value across all three fence kinds so a fence
/// run reads as one continuous piece.
const FENCE_COLOR: [f32; 3] = [0.42, 0.32, 0.20];
/// Bottom rail sits low, top rail near the top of the 60-tall collider.
/// The gap between them (~35 units) is the see-through part.
const FENCE_BOTTOM_Y: f32 = 12.0;
const FENCE_TOP_Y: f32 = 48.0;

fn is_fence(k: PropKind) -> bool {
    matches!(k, PropKind::Fence | PropKind::FenceNS | PropKind::FenceEW)
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
    // Dev grid.
    //
    // Two layers so wall placement can be eyeballed unambiguously:
    //   * MAJOR (80 units, brighter) — CDDA cell boundaries. Walls
    //     sit either on these lines (outer edges) or exactly between
    //     them (dividers at cell centre).
    //   * MINOR (40 units, fainter) — cell centres. Odd-width mapgens
    //     (cx = width/2 = 2.5) offset cell corners by 40 from any
    //     80-multiple, so the minor lines also hit the corners for
    //     those buildings. Together the two layers show every place
    //     a wall segment can legitimately land.
    //
    // Snapped to the grid so lines stay stationary as the player moves.
    const GRID_HALF: f32 = 2000.0;
    const CELL_STEP: f32 = 80.0;
    const HALF_STEP: f32 = 40.0;
    const MINOR_COLOR: [f32; 3] = [0.13, 0.15, 0.19];
    const MAJOR_COLOR: [f32; 3] = [0.22, 0.24, 0.30];
    const GRID_THICK: f32 = 2.0;
    let px_major = (snap.player.x / CELL_STEP).round() * CELL_STEP;
    let pz_major = (snap.player.z / CELL_STEP).round() * CELL_STEP;
    let n_major = (GRID_HALF / CELL_STEP) as i32;
    for i in -n_major..=n_major {
        let x = px_major + (i as f32) * CELL_STEP;
        instances.push(SceneInstance {
            pos: [x, 0.1, pz_major],
            color: MAJOR_COLOR,
            scale: [GRID_THICK, 1.0, GRID_HALF * 2.0],
        });
        let z = pz_major + (i as f32) * CELL_STEP;
        instances.push(SceneInstance {
            pos: [px_major, 0.1, z],
            color: MAJOR_COLOR,
            scale: [GRID_HALF * 2.0, 1.0, GRID_THICK],
        });
    }
    // Minor grid at cell centres — offset by half a cell from the
    // major grid so it never overlaps with a major line.
    let px_minor = px_major + HALF_STEP;
    let pz_minor = pz_major + HALF_STEP;
    for i in -n_major..=n_major {
        let x = px_minor + (i as f32) * CELL_STEP;
        instances.push(SceneInstance {
            pos: [x, 0.1, pz_minor],
            color: MINOR_COLOR,
            scale: [GRID_THICK, 1.0, GRID_HALF * 2.0],
        });
        let z = pz_minor + (i as f32) * CELL_STEP;
        instances.push(SceneInstance {
            pos: [px_minor, 0.1, z],
            color: MINOR_COLOR,
            scale: [GRID_HALF * 2.0, 1.0, GRID_THICK],
        });
    }
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
    // Trees are no longer emitted here — they render through the mesh
    // pipeline (tapered trunk mesh + compound-instanced icosahedron
    // canopy). See `snapshot_to_mesh_instances` and `game/docs/RENDER.md`.
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
    let roof_half = cdda::CDDA_TILE / 2.0;
    let overhead = snap.structures.iter().find(|(p, k, _, _)| {
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
    let overhead_pos = overhead.map(|(p, _, _, _)| *p).unwrap_or(snap.player);
    // The isometric camera looks from +x,+z, so nearer props have a
    // larger x+z. When inside, hide this building's roof AND its
    // camera-facing walls (in front of the player) so the interior
    // shows; the far walls stay as a backdrop.
    let player_depth = snap.player.x + snap.player.z;

    // Structure props (walls, furniture, roof). Size comes from the
    // kind; colour is the prop's own tint (walls by material) or the
    // kind default. Ground props sit on the floor (base at y=0);
    // elevated props (the roof) carry their height in pos.y.
    for (pos, kind, tint, size_override) in &snap.structures {
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
        let (default_color, default_scale) = prop_appearance(*kind);
        let color = tint.unwrap_or(default_color);
        let scale = size_override
            .map(|s| [s.x, s.y, s.z])
            .unwrap_or(default_scale);
        if is_fence(*kind) {
            // Bottom + top rail, gap between. Two thin bars per fence cell.
            instances.push(SceneInstance {
                pos: [pos.x, FENCE_BOTTOM_Y, pos.z],
                color,
                scale,
            });
            instances.push(SceneInstance {
                pos: [pos.x, FENCE_TOP_Y, pos.z],
                color,
                scale,
            });
            // Vertical post at each end of the cell along the fence's
            // axis, so the run reads as posts + rails, not floating bars.
            let post_scale = [8.0, 60.0, 8.0];
            let (dx, dz) = match *kind {
                PropKind::FenceEW => (40.0, 0.0),
                PropKind::FenceNS => (0.0, 40.0),
                _ => (0.0, 0.0),
            };
            instances.push(SceneInstance {
                pos: [pos.x - dx, post_scale[1] * 0.5, pos.z - dz],
                color,
                scale: post_scale,
            });
            instances.push(SceneInstance {
                pos: [pos.x + dx, post_scale[1] * 0.5, pos.z + dz],
                color,
                scale: post_scale,
            });
            continue;
        }
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
    let roof_half = cdda::CDDA_TILE / 2.0;
    let overhead = snap.structures.iter().find(|(p, k, _, _)| {
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
    for (pos, kind, tint, size_override) in &snap.structures {
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
        let (default_color, default_scale) = prop_appearance(*kind);
        let color = tint.unwrap_or(default_color);
        let scale = size_override
            .map(|s| [s.x, s.y, s.z])
            .unwrap_or(default_scale);
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
    for (pos, kind, tint, size_override) in &snap.structures {
        if !kind.is_window() {
            continue;
        }
        let (default_color, default_scale) = prop_appearance(*kind);
        let color = tint.unwrap_or(default_color);
        let scale = size_override
            .map(|s| [s.x, s.y, s.z])
            .unwrap_or(default_scale);
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
    fn instance_attrs_match_the_repr_c_struct() {
        // The source-of-truth const must agree with the actual repr(C)
        // byte layout it claims to describe: four vec3s at 0/12/24/36 and
        // a 48-byte stride. If MeshInstance changes, this fails first.
        assert_eq!(INSTANCE_ATTRS.len(), 4);
        // pos/color/scale are vec3 at 0/12/24; axis is a vec4 at 36.
        let expected = [
            (3u32, 0u64, "float32x3"),
            (4, 12, "float32x3"),
            (5, 24, "float32x3"),
            (6, 36, "float32x4"),
        ];
        for (a, (loc, off, fmt)) in INSTANCE_ATTRS.iter().zip(expected) {
            assert_eq!(a.location, loc);
            assert_eq!(a.offset, off);
            assert_eq!(a.format, fmt);
        }
        assert_eq!(INSTANCE_STRIDE, 52);
        assert_eq!(INSTANCE_STRIDE, std::mem::size_of::<MeshInstance>() as u64);
    }

    #[test]
    fn web_shim_mesh_instance_layout_matches_this_const() {
        // The ONE hand-written copy of the instance layout is the JS shim.
        // seer renders the native path, so native drift shows in a frame;
        // nothing exercises the JS offsets but a real browser — so hold
        // web/src/main.ts to INSTANCE_ATTRS here. Embedded at compile time
        // so this runs in the fast game-tests gate, not just in a browser.
        let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/main.ts"));
        // Scope to the mesh pipeline's INSTANCE attribute array (locations
        // 3-6), not the vertex one (0-2) or any other pipeline's.
        let fn_at = src
            .find("game_gpu_render_pipeline_create_mesh")
            .expect("mesh pipeline factory in main.ts");
        let inst_at = fn_at
            + src[fn_at..]
                .find("stepMode: 'instance'")
                .expect("instance buffer in the mesh pipeline");
        let start = inst_at
            + src[inst_at..].find("attributes: [").expect("attributes array");
        let end = start + src[start..].find(']').expect("attributes array close");
        let parsed = parse_js_attrs(&src[start..end]);
        assert_eq!(
            parsed.len(),
            INSTANCE_ATTRS.len(),
            "JS mesh-instance attribute count drifted from INSTANCE_ATTRS"
        );
        for (want, got) in INSTANCE_ATTRS.iter().zip(&parsed) {
            assert_eq!(got.0, want.location, "JS shaderLocation drifted");
            assert_eq!(got.1, want.offset, "JS offset drifted at location {}", want.location);
            assert_eq!(got.2, want.format, "JS format drifted at location {}", want.location);
        }
    }

    /// Parse `{ shaderLocation: N, offset: M, format: 'F' }` entries, in
    /// order, from a JS attributes-array slice.
    fn parse_js_attrs(block: &str) -> Vec<(u32, u64, String)> {
        fn uint(s: &str) -> u64 {
            s.trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap()
        }
        let mut out = Vec::new();
        let mut rest = block;
        while let Some(p) = rest.find("shaderLocation:") {
            rest = &rest[p + "shaderLocation:".len()..];
            let loc = uint(rest) as u32;
            let o = rest.find("offset:").expect("offset in attr");
            let off = uint(&rest[o + "offset:".len()..]);
            let f = rest.find("format:").expect("format in attr");
            let fs = &rest[f + "format:".len()..];
            let q0 = fs.find('\'').unwrap() + 1;
            let q1 = fs[q0..].find('\'').unwrap() + q0;
            out.push((loc, off, fs[q0..q1].to_string()));
        }
        out
    }

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
            structures: vec![(Vec3::new(0.0, 220.0, 0.0), PropKind::Roof, None, None)],
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

    fn structure_at(
        x: f32,
        y: f32,
        z: f32,
        kind: PropKind,
    ) -> (Vec3, PropKind, Option<[f32; 3]>, Option<Vec3>) {
        (Vec3::new(x, y, z), kind, None, None)
    }

    #[test]
    fn cut_away_stays_within_the_building_you_are_actually_under() {
        // Player is inside building A (roof at origin). Building B sits
        // ~900 units east but is a separate building (its own roof).
        // B's camera-facing wall must stay solid.
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
                (Vec3::new(0.0, 220.0, 0.0), PropKind::Roof, None, None), // overhead → inside
                (Vec3::new(300.0, 0.0, 300.0), PropKind::Wall, None, None), // camera-facing (x+z>0)
                (Vec3::new(-300.0, 0.0, -300.0), PropKind::Wall, None, None), // far side
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
                (Vec3::new(0.0, 220.0, 0.0), PropKind::Roof, None, None), // overhead → inside
                (Vec3::new(300.0, 0.0, 300.0), PropKind::Wall, None, None), // camera-facing (x+z>0)
                (Vec3::new(-300.0, 0.0, -300.0), PropKind::Wall, None, None), // far side (still opaque)
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
                (Vec3::new(0.0, 220.0, 0.0), PropKind::Roof, None, None),
                (Vec3::new(300.0, 0.0, 300.0), PropKind::Wall, None, None),
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
                (Vec3::new(300.0, 0.0, 0.0), PropKind::WallNS, None, None),
                (Vec3::new(300.0, 0.0, 80.0), PropKind::WindowNS, Some([0.5, 0.68, 0.82]), None),
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
