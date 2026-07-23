// Wasm-side renderer — uses hand-wired gpu_web wrappers. Mirrors
// render.rs's cube pipeline shape but bound to a live canvas via
// GameRenderTarget instead of the offscreen texture + PNG readback.
//
// Persistent state (pipeline, target, vertex buf, camera uniform,
// bind group) lives in a thread_local — created once by init(),
// used by frame() every tick.

use std::cell::RefCell;

use crate::dpad;
use crate::gpu_web;
use crate::hud;
use crate::obs;
use crate::scene::{
    self, GpuVertex, MeshInstance, SceneCamera, SceneInstance, as_bytes, cube_geometry,
};
use crate::shaders::{
    GHOST_SHADER_WGSL, GLASS_SHADER_WGSL, GROUND_SHADER_WGSL, LEAF_SHADER_WGSL, MESH_SHADER_WGSL,
    SHADER_WGSL,
    UI_SHADER_WGSL, WALL_SHADER_WGSL,
};
use crate::tree_mesh::{self, MeshVertex};

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuCamera {
    view_proj: [[f32; 4]; 4],
    /// `x` = elapsed seconds driving leaf-wind sway; `yzw` spare. Mirrors
    /// `Camera.wind` in the mesh/leaf WGSL and the native `GpuCamera`.
    wind: [f32; 4],
}

struct RenderWebState {
    target: gpu_web::GameRenderTarget,
    pipeline: gpu_web::GameRenderPipeline,
    bind_group: gpu_web::GameBindGroup,
    camera_buf: gpu_web::GameBuffer,
    vertex_buf: gpu_web::GameBuffer,
    instance_buf: Option<gpu_web::GameBuffer>,
    instance_capacity: usize,
    vertex_count: u32,
    // Glass pass — translucent pipeline + its own growable instance
    // buffer (windows only), drawn between the world and the UI.
    glass_pipeline: gpu_web::GameRenderPipeline,
    glass_instance_buf: Option<gpu_web::GameBuffer>,
    glass_instance_capacity: usize,
    // Ghost pass — cut-away walls + roof at low alpha, drawn after
    // glass so the outline sits on top.
    ghost_pipeline: gpu_web::GameRenderPipeline,
    ghost_instance_buf: Option<gpu_web::GameBuffer>,
    ghost_instance_capacity: usize,
    // UI overlay — pipeline + growable instance buffer for D-pad + HUD.
    ui_pipeline: gpu_web::GameUiPipeline,
    ui_instance_buf: Option<gpu_web::GameBuffer>,
    ui_instance_capacity: usize,
    // Mesh pipeline — the wood (docs/TREES.md) drawn as one instanced draw
    // per species using that species' mesh from
    // `tree_surface::species_wood_mesh`, plus the instanced canopy
    // elements. Wood mesh generation runs at most 8 times over the
    // process (one per species), not per tree — the design that makes
    // wasm shippable.
    //
    // Per-species buffers are created on first sight of each species
    // and kept for the process lifetime (`SpeciesGpuBufs` in
    // `wood_species`). Vertex/index buffers hold the CPU mesh (never
    // re-uploaded); the instance buffer grows with the species'
    // per-frame tree count.
    mesh_pipeline: gpu_web::GameRenderPipeline,
    leaf_pipeline: gpu_web::GameRenderPipeline,
    // Ground pipeline: the solid terrain surface + the world-anchored grid
    // painted in its shader (no grid geometry any more).
    ground_pipeline: gpu_web::GameRenderPipeline,
    wood_species: Vec<SpeciesGpuBufs>,
    canopy_vertex_buf: gpu_web::GameBuffer,
    canopy_index_buf: gpu_web::GameBuffer,
    canopy_index_count: u32,
    canopy_instance_buf: Option<gpu_web::GameBuffer>,
    canopy_instance_capacity: usize,
    // The solid terrain surface — one mesh, player-centred; regenerated +
    // re-uploaded only when the player crosses a cell (snap-key cache).
    surface_vertex_buf: Option<gpu_web::GameBuffer>,
    surface_index_buf: Option<gpu_web::GameBuffer>,
    surface_instance_buf: Option<gpu_web::GameBuffer>,
    surface_vertex_capacity: usize,
    surface_index_count: u32,
    surface_snap_key: Option<(i32, i32)>,
    // Walls-on-mesh (RENDER.md slice 4): baked building walls, uploaded
    // per colour part when the player crosses a chunk; drawn each frame
    // with the cut-away index range (no re-upload).
    wall_pipeline: gpu_web::GameRenderPipeline,
    walls_gpu: Vec<WallPartGpu>,
    walls_chunk_key: Option<(i32, i32)>,
}

/// One uploaded wall colour part: geometry + instance buffers, the
/// cut-away split, and enough placement data to compute the per-frame
/// draw range on the CPU.
struct WallPartGpu {
    vertex_buf: gpu_web::GameBuffer,
    index_buf: gpu_web::GameBuffer,
    instance_buf: gpu_web::GameBuffer,
    index_total: u32,
    near_start: u32,
    near_depths: Vec<f32>,
    anchor: [f32; 2],
}

/// One species' GPU resources on the wasm path. The vertex + index
/// buffers hold the species wood mesh; `mesh_generation` tracks the
/// `tree_surface::mesh_generation()` value at last upload so the
/// system re-uploads on a tune-HUD invalidation. Instance buffer grows
/// with the per-frame tree count for that species.
struct SpeciesGpuBufs {
    key: usize,
    mesh_generation: u32,
    vertex_buf: gpu_web::GameBuffer,
    vertex_capacity: usize,
    index_buf: gpu_web::GameBuffer,
    index_capacity: usize,
    index_count: u32,
    instance_buf: Option<gpu_web::GameBuffer>,
    instance_capacity: usize,
}

thread_local! {
    static STATE: RefCell<Option<RenderWebState>> = const { RefCell::new(None) };
}

/// Called once, when gpu_web::status() == Ready. Creates shader,
/// layouts, pipeline, persistent buffers, canvas target. Returns true
/// on success — false means the browser refused something and the
/// caller should skip rendering going forward.
pub fn init(canvas_id: &str) -> bool {
    let target = gpu_web::GameRenderTarget::configure(
        canvas_id,
        gpu_web::color_format::BGRA8UNORM,
        gpu_web::depth_format::DEPTH32FLOAT,
    );
    let shader = gpu_web::GameShaderModule::create(SHADER_WGSL, "render_web.shader");
    let bg_layout = gpu_web::GameBindGroupLayout::create_uniform("render_web.bgl");
    let (Some(target), Some(shader), Some(bg_layout)) = (target, shader, bg_layout) else {
        obs::emit("[render_web] init: target/shader/bgl null");
        return false;
    };

    let camera_buf = gpu_web::GameBuffer::create(
        std::mem::size_of::<GpuCamera>() as u32,
        gpu_web::usage::UNIFORM | gpu_web::usage::COPY_DST,
        "render_web.camera",
    );
    let vertices = cube_geometry();
    let vertex_size = std::mem::size_of_val(&vertices[..]) as u32;
    let vertex_buf = gpu_web::GameBuffer::create(
        vertex_size,
        gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
        "render_web.vertex",
    );
    let (Some(camera_buf), Some(vertex_buf)) = (camera_buf, vertex_buf) else {
        obs::emit("[render_web] init: camera_buf/vertex_buf null");
        return false;
    };
    vertex_buf.write(as_bytes(&vertices));

    let bind_group = gpu_web::GameBindGroup::create(&bg_layout, &camera_buf, "render_web.bg");
    let pl_layout = gpu_web::GamePipelineLayout::create(&bg_layout, "render_web.pl");
    let (Some(bind_group), Some(pl_layout)) = (bind_group, pl_layout) else {
        obs::emit("[render_web] init: bind_group/pl_layout null");
        return false;
    };

    let pipeline = gpu_web::GameRenderPipeline::create_cube(
        &pl_layout,
        &shader,
        std::mem::size_of::<GpuVertex>() as u32,
        std::mem::size_of::<SceneInstance>() as u32,
        gpu_web::color_format::BGRA8UNORM,
        gpu_web::depth_format::DEPTH32FLOAT,
        "render_web.pipeline",
    );
    let Some(pipeline) = pipeline else {
        obs::emit("[render_web] init: pipeline null");
        return false;
    };

    // UI overlay: separate pipeline (no vertex buffer, no depth),
    // draws screen-space quads on top of the world. Shares the
    // camera bind group with the world pipeline for pipeline-layout
    // reuse — the UI shader declares but doesn't use the uniform.
    let ui_shader = gpu_web::GameShaderModule::create(UI_SHADER_WGSL, "render_web.ui.shader");
    let Some(ui_shader) = ui_shader else {
        obs::emit("[render_web] init: ui shader null");
        return false;
    };
    let ui_pipeline = gpu_web::GameUiPipeline::create(
        &pl_layout,
        &ui_shader,
        std::mem::size_of::<dpad::DpadInstance>() as u32,
        gpu_web::color_format::BGRA8UNORM,
        "render_web.ui.pipeline",
    );
    let Some(ui_pipeline) = ui_pipeline else {
        obs::emit("[render_web] init: ui pipeline null");
        return false;
    };
    // Glass pass: a translucent cube pipeline sharing the world's
    // pipeline layout + vertex/instance layout, alpha-blended with
    // depth-write off (see scene::GLASS_SHADER_WGSL).
    let glass_shader = gpu_web::GameShaderModule::create(GLASS_SHADER_WGSL, "render_web.glass.shader");
    let Some(glass_shader) = glass_shader else {
        obs::emit("[render_web] init: glass shader null");
        return false;
    };
    let glass_pipeline = gpu_web::GameRenderPipeline::create_glass(
        &pl_layout,
        &glass_shader,
        std::mem::size_of::<GpuVertex>() as u32,
        std::mem::size_of::<SceneInstance>() as u32,
        gpu_web::color_format::BGRA8UNORM,
        gpu_web::depth_format::DEPTH32FLOAT,
        "render_web.glass.pipeline",
    );
    let Some(glass_pipeline) = glass_pipeline else {
        obs::emit("[render_web] init: glass pipeline null");
        return false;
    };
    // Ghost pipeline: shares the pipeline layout + world's vertex layout,
    // its own shader emits a low constant alpha for cut-away outlines.
    let ghost_shader = gpu_web::GameShaderModule::create(GHOST_SHADER_WGSL, "render_web.ghost.shader");
    let Some(ghost_shader) = ghost_shader else {
        obs::emit("[render_web] init: ghost shader null");
        return false;
    };
    let ghost_pipeline = gpu_web::GameRenderPipeline::create_ghost(
        &pl_layout,
        &ghost_shader,
        std::mem::size_of::<GpuVertex>() as u32,
        std::mem::size_of::<SceneInstance>() as u32,
        gpu_web::color_format::BGRA8UNORM,
        gpu_web::depth_format::DEPTH32FLOAT,
        "render_web.ghost.pipeline",
    );
    let Some(ghost_pipeline) = ghost_pipeline else {
        obs::emit("[render_web] init: ghost pipeline null");
        return false;
    };

    // Mesh pipeline. Its shader adds a UV attribute (day-one commitment
    // for damage textures) and does the inverse-transpose normal fix
    // for non-uniform per-instance scale — see MESH_SHADER_WGSL.
    let mesh_shader = gpu_web::GameShaderModule::create(MESH_SHADER_WGSL, "render_web.mesh.shader");
    let Some(mesh_shader) = mesh_shader else {
        obs::emit("[render_web] init: mesh shader null");
        return false;
    };
    let mesh_pipeline = gpu_web::GameRenderPipeline::create_mesh(
        &pl_layout,
        &mesh_shader,
        std::mem::size_of::<MeshVertex>() as u32,
        std::mem::size_of::<MeshInstance>() as u32,
        gpu_web::color_format::BGRA8UNORM,
        gpu_web::depth_format::DEPTH32FLOAT,
        "render_web.mesh.pipeline",
    );
    let Some(mesh_pipeline) = mesh_pipeline else {
        obs::emit("[render_web] init: mesh pipeline null");
        return false;
    };
    // Leaf pipeline: same vertex/instance layout, a fragment shader that
    // carves a leaf silhouette from the card (LEAF_SHADER_WGSL). Only the
    // canopy draw uses it; trunks/branches keep the mesh pipeline.
    let leaf_shader = gpu_web::GameShaderModule::create(LEAF_SHADER_WGSL, "render_web.leaf.shader");
    let Some(leaf_shader) = leaf_shader else {
        obs::emit("[render_web] init: leaf shader null");
        return false;
    };
    let leaf_pipeline = gpu_web::GameRenderPipeline::create_mesh(
        &pl_layout,
        &leaf_shader,
        std::mem::size_of::<MeshVertex>() as u32,
        std::mem::size_of::<MeshInstance>() as u32,
        gpu_web::color_format::BGRA8UNORM,
        gpu_web::depth_format::DEPTH32FLOAT,
        "render_web.leaf.pipeline",
    );
    let Some(leaf_pipeline) = leaf_pipeline else {
        obs::emit("[render_web] init: leaf pipeline null");
        return false;
    };
    // Ground pipeline: same layout, a shader that Lambert-shades the solid
    // terrain surface and paints the faint WORLD-ANCHORED grid on it.
    let ground_shader =
        gpu_web::GameShaderModule::create(GROUND_SHADER_WGSL, "render_web.ground.shader");
    let Some(ground_shader) = ground_shader else {
        obs::emit("[render_web] init: ground shader null");
        return false;
    };
    let ground_pipeline = gpu_web::GameRenderPipeline::create_mesh(
        &pl_layout,
        &ground_shader,
        std::mem::size_of::<MeshVertex>() as u32,
        std::mem::size_of::<MeshInstance>() as u32,
        gpu_web::color_format::BGRA8UNORM,
        gpu_web::depth_format::DEPTH32FLOAT,
        "render_web.ground.pipeline",
    );
    let Some(ground_pipeline) = ground_pipeline else {
        obs::emit("[render_web] init: ground pipeline null");
        return false;
    };
    // Wall pipeline: mesh layout, plain-Lambert fragment (no bark).
    let wall_shader = gpu_web::GameShaderModule::create(WALL_SHADER_WGSL, "render_web.wall.shader");
    let Some(wall_shader) = wall_shader else {
        obs::emit("[render_web] init: wall shader null");
        return false;
    };
    let wall_pipeline = gpu_web::GameRenderPipeline::create_mesh(
        &pl_layout,
        &wall_shader,
        std::mem::size_of::<MeshVertex>() as u32,
        std::mem::size_of::<MeshInstance>() as u32,
        gpu_web::color_format::BGRA8UNORM,
        gpu_web::depth_format::DEPTH32FLOAT,
        "render_web.wall.pipeline",
    );
    let Some(wall_pipeline) = wall_pipeline else {
        obs::emit("[render_web] init: wall pipeline null");
        return false;
    };

    // Canopy: baked once at init (unit leaf card). Wood buffers grow
    // lazily on the first non-empty snapshot.
    let (canopy_verts, canopy_indices) = tree_mesh::leaf_quad_mesh();
    let canopy_vertex_buf = gpu_web::GameBuffer::create(
        std::mem::size_of_val(&canopy_verts[..]) as u32,
        gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
        "render_web.mesh.canopy.vertex",
    );
    let canopy_index_buf = gpu_web::GameBuffer::create(
        std::mem::size_of_val(&canopy_indices[..]) as u32,
        gpu_web::usage::INDEX | gpu_web::usage::COPY_DST,
        "render_web.mesh.canopy.index",
    );
    let (Some(canopy_vertex_buf), Some(canopy_index_buf)) =
        (canopy_vertex_buf, canopy_index_buf)
    else {
        obs::emit("[render_web] init: canopy buffer null");
        return false;
    };
    canopy_vertex_buf.write(as_bytes(&canopy_verts));
    canopy_index_buf.write(as_bytes(&canopy_indices));
    let canopy_index_count = canopy_indices.len() as u32;

    STATE.with(|c| {
        *c.borrow_mut() = Some(RenderWebState {
            target,
            pipeline,
            bind_group,
            camera_buf,
            vertex_buf,
            instance_buf: None,
            instance_capacity: 0,
            vertex_count: vertices.len() as u32,
            glass_pipeline,
            glass_instance_buf: None,
            glass_instance_capacity: 0,
            ghost_pipeline,
            ghost_instance_buf: None,
            ghost_instance_capacity: 0,
            ui_pipeline,
            ui_instance_buf: None,
            ui_instance_capacity: 0,
            mesh_pipeline,
            leaf_pipeline,
            ground_pipeline,
            wood_species: Vec::new(),
            canopy_vertex_buf,
            canopy_index_buf,
            canopy_index_count,
            canopy_instance_buf: None,
            canopy_instance_capacity: 0,
            surface_vertex_buf: None,
            surface_index_buf: None,
            surface_instance_buf: None,
            surface_vertex_capacity: 0,
            surface_index_count: 0,
            surface_snap_key: None,
            wall_pipeline,
            walls_gpu: Vec::new(),
            walls_chunk_key: None,
        });
    });
    obs::emit(&format!(
        "[render_web] init OK — {} verts, canvas={canvas_id}",
        vertices.len()
    ));
    true
}

/// Per-frame render. Writes camera + instances; recreates the instance
/// buffer if the scene grew past its capacity.
pub fn frame(camera: &SceneCamera, instances: &[SceneInstance], time: f32) -> u32 {
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2; // not initialized
        };

        let tp = crate::tune::get();
        let gpu_camera = GpuCamera {
            view_proj: camera.view_proj(),
            wind: [time, tp.wind_amp, tp.wind_speed, tp.wind_leaf_mult],
        };
        state.camera_buf.write(as_bytes(std::slice::from_ref(&gpu_camera)));

        if state.instance_buf.is_none() || instances.len() > state.instance_capacity {
            let new_cap = instances.len().max(16);
            let new_size = (new_cap * std::mem::size_of::<SceneInstance>()) as u32;
            state.instance_buf = gpu_web::GameBuffer::create(
                new_size,
                gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                "render_web.instance",
            );
            state.instance_capacity = new_cap;
        }
        let Some(instance_buf) = state.instance_buf.as_ref() else {
            return 3;
        };
        instance_buf.write(as_bytes(instances));

        gpu_web::render_frame(
            &state.target,
            &state.pipeline,
            &state.bind_group,
            &state.vertex_buf,
            instance_buf,
            state.vertex_count,
            instances.len() as u32,
            [0.03, 0.05, 0.09],
        )
    })
}

/// Glass pass — draws the translucent window panes on top of the
/// opaque world, loading (not clearing) colour + depth so they blend
/// over it and are occluded by nearer opaque geometry. Grows its own
/// instance buffer like the world pass. A no-op (success) when there
/// are no panes.
pub fn frame_glass(instances: &[SceneInstance]) -> u32 {
    if instances.is_empty() {
        return 0;
    }
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2;
        };
        if state.glass_instance_buf.is_none() || instances.len() > state.glass_instance_capacity {
            let new_cap = instances.len().max(16);
            let new_size = (new_cap * std::mem::size_of::<SceneInstance>()) as u32;
            state.glass_instance_buf = gpu_web::GameBuffer::create(
                new_size,
                gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                "render_web.glass.instance",
            );
            state.glass_instance_capacity = new_cap;
        }
        let Some(glass_buf) = state.glass_instance_buf.as_ref() else {
            return 3;
        };
        glass_buf.write(as_bytes(instances));
        gpu_web::render_glass(
            &state.target,
            &state.glass_pipeline,
            &state.bind_group,
            &state.vertex_buf,
            glass_buf,
            state.vertex_count,
            instances.len() as u32,
        )
    })
}

/// Ghost pass — cut-away walls + roof at low alpha, drawn after the
/// glass pass so the outline sits on top. Grows its own instance
/// buffer as the ghost set changes with the player's inside/outside
/// state. A no-op success when there are no ghosts.
pub fn frame_ghost(instances: &[SceneInstance]) -> u32 {
    if instances.is_empty() {
        return 0;
    }
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2;
        };
        if state.ghost_instance_buf.is_none() || instances.len() > state.ghost_instance_capacity {
            let new_cap = instances.len().max(16);
            let new_size = (new_cap * std::mem::size_of::<SceneInstance>()) as u32;
            state.ghost_instance_buf = gpu_web::GameBuffer::create(
                new_size,
                gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                "render_web.ghost.instance",
            );
            state.ghost_instance_capacity = new_cap;
        }
        let Some(ghost_buf) = state.ghost_instance_buf.as_ref() else {
            return 3;
        };
        ghost_buf.write(as_bytes(instances));
        gpu_web::render_ghost(
            &state.target,
            &state.ghost_pipeline,
            &state.bind_group,
            &state.vertex_buf,
            ghost_buf,
            state.vertex_count,
            instances.len() as u32,
        )
    })
}

/// Mesh pass — one instanced draw per species (each drawing that
/// species' species wood mesh across its trees), then the canopy
/// dispatch. Canonical-per-species geometry means the vertex + index
/// buffers are uploaded ONCE per species (on first sight) and never
/// touched again; only the small instance buffers change per frame.
///
/// This is the shippable shape: wood cost across the world is O(species)
/// unique meshes generated + O(trees) `MeshInstance` writes per frame —
/// no per-tree generation, no merge, no growing merged buffer.
pub fn frame_mesh(
    wood_by_species: &[(&'static crate::tree_mesh::TreeSpecies, Vec<MeshInstance>)],
    canopy: &[MeshInstance],
) -> u32 {
    let has_wood = wood_by_species.iter().any(|(_, v)| !v.is_empty());
    if !has_wood && canopy.is_empty() {
        return 0;
    }
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2;
        };

        // Wood: one draw per species. Upload the species' vertex+index
        // buffers on first sight (cached forever), grow its instance
        // buffer to fit this frame's tree count for that species.
        let current_gen = crate::tree_surface::mesh_generation();
        for (sp, instances) in wood_by_species {
            if instances.is_empty() {
                continue;
            }
            let key = *sp as *const crate::tree_mesh::TreeSpecies as usize;
            let idx = state.wood_species.iter().position(|s| s.key == key);
            let idx = match idx {
                Some(i) => i,
                None => {
                    let mesh = crate::tree_surface::species_wood_mesh(sp);
                    let vsize = std::mem::size_of_val(&mesh.0[..]) as u32;
                    let isize_bytes = std::mem::size_of_val(&mesh.1[..]) as u32;
                    let vertex_buf = gpu_web::GameBuffer::create(
                        vsize,
                        gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                        "render_web.mesh.wood.vertex",
                    );
                    let index_buf = gpu_web::GameBuffer::create(
                        isize_bytes,
                        gpu_web::usage::INDEX | gpu_web::usage::COPY_DST,
                        "render_web.mesh.wood.index",
                    );
                    let (Some(vertex_buf), Some(index_buf)) = (vertex_buf, index_buf) else {
                        return 4;
                    };
                    vertex_buf.write(as_bytes(&mesh.0));
                    index_buf.write(as_bytes(&mesh.1));
                    state.wood_species.push(SpeciesGpuBufs {
                        key,
                        mesh_generation: current_gen,
                        vertex_buf,
                        vertex_capacity: vsize as usize,
                        index_buf,
                        index_capacity: isize_bytes as usize,
                        index_count: mesh.1.len() as u32,
                        instance_buf: None,
                        instance_capacity: 0,
                    });
                    state.wood_species.len() - 1
                }
            };
            // Regenerate + re-upload when the mesh generation advanced
            // (a tune-HUD wood-shape slider committed).
            if state.wood_species[idx].mesh_generation != current_gen {
                let mesh = crate::tree_surface::species_wood_mesh(sp);
                let vsize = std::mem::size_of_val(&mesh.0[..]);
                let isize_bytes = std::mem::size_of_val(&mesh.1[..]);
                // If the new mesh is bigger than the existing buffer,
                // reallocate; else reuse in place.
                if vsize > state.wood_species[idx].vertex_capacity {
                    let buf = gpu_web::GameBuffer::create(
                        vsize as u32,
                        gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                        "render_web.mesh.wood.vertex",
                    );
                    let Some(buf) = buf else { return 6 };
                    state.wood_species[idx].vertex_buf = buf;
                    state.wood_species[idx].vertex_capacity = vsize;
                }
                if isize_bytes > state.wood_species[idx].index_capacity {
                    let buf = gpu_web::GameBuffer::create(
                        isize_bytes as u32,
                        gpu_web::usage::INDEX | gpu_web::usage::COPY_DST,
                        "render_web.mesh.wood.index",
                    );
                    let Some(buf) = buf else { return 7 };
                    state.wood_species[idx].index_buf = buf;
                    state.wood_species[idx].index_capacity = isize_bytes;
                }
                state.wood_species[idx].vertex_buf.write(as_bytes(&mesh.0));
                state.wood_species[idx].index_buf.write(as_bytes(&mesh.1));
                state.wood_species[idx].index_count = mesh.1.len() as u32;
                state.wood_species[idx].mesh_generation = current_gen;
            }
            let sb = &mut state.wood_species[idx];
            if sb.instance_buf.is_none() || instances.len() > sb.instance_capacity {
                let new_cap = instances.len().max(32);
                let new_size = (new_cap * std::mem::size_of::<MeshInstance>()) as u32;
                sb.instance_buf = gpu_web::GameBuffer::create(
                    new_size,
                    gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                    "render_web.mesh.wood.instance",
                );
                sb.instance_capacity = new_cap;
            }
            let Some(inst_buf) = sb.instance_buf.as_ref() else {
                return 5;
            };
            inst_buf.write(as_bytes(instances));
            let r = gpu_web::render_mesh(
                &state.target,
                &state.mesh_pipeline,
                &state.bind_group,
                &sb.vertex_buf,
                &sb.index_buf,
                inst_buf,
                sb.index_count,
                instances.len() as u32,
                0,
            );
            if r != 0 {
                return r;
            }
        }

        // Canopy: one dispatch, one instance buffer that grows with the
        // canopy element count.
        if !canopy.is_empty() {
            if state.canopy_instance_buf.is_none() || canopy.len() > state.canopy_instance_capacity {
                let new_cap = canopy.len().max(256);
                let new_size = (new_cap * std::mem::size_of::<MeshInstance>()) as u32;
                state.canopy_instance_buf = gpu_web::GameBuffer::create(
                    new_size,
                    gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                    "render_web.mesh.canopy.instance",
                );
                state.canopy_instance_capacity = new_cap;
            }
            let Some(canopy_buf) = state.canopy_instance_buf.as_ref() else {
                return 6;
            };
            canopy_buf.write(as_bytes(canopy));
            let r = gpu_web::render_mesh(
                &state.target,
                &state.leaf_pipeline,
                &state.bind_group,
                &state.canopy_vertex_buf,
                &state.canopy_index_buf,
                canopy_buf,
                state.canopy_index_count,
                canopy.len() as u32,
                0,
            );
            if r != 0 {
                return r;
            }
        }
        0
    })
}

/// Solid terrain surface — one mesh, one identity instance, mesh pipeline.
/// The patch is player-centred, so the vertex buffer is re-uploaded each
/// frame; the index buffer + identity instance are constant, written once.
/// See docs/TERRAIN.md, Slice 6/8.
pub fn frame_surface(px: f32, pz: f32) -> u32 {
    let key = scene::surface_snap(px, pz);
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2;
        };
        // Regenerate + re-upload only when the player crossed a cell.
        if state.surface_snap_key != Some(key) {
            let surface = scene::terrain_surface_mesh(px, pz);
            if surface.indices.is_empty() {
                return 0;
            }
            let vbytes = std::mem::size_of_val(&surface.verts[..]);
            if state.surface_vertex_buf.is_none() || vbytes > state.surface_vertex_capacity {
                state.surface_vertex_buf = gpu_web::GameBuffer::create(
                    vbytes as u32,
                    gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                    "render_web.mesh.surface.vertex",
                );
                state.surface_vertex_capacity = vbytes;
            }
            // Index topology + identity instance never change — write once.
            if state.surface_index_buf.is_none() {
                let ibuf = gpu_web::GameBuffer::create(
                    std::mem::size_of_val(&surface.indices[..]) as u32,
                    gpu_web::usage::INDEX | gpu_web::usage::COPY_DST,
                    "render_web.mesh.surface.index",
                );
                let Some(ibuf) = ibuf else { return 4 };
                ibuf.write(as_bytes(&surface.indices));
                state.surface_index_count = surface.indices.len() as u32;
                state.surface_index_buf = Some(ibuf);

                let inst = [MeshInstance {
                    pos: [0.0, 0.0, 0.0],
                    color: [0.20, 0.26, 0.15],
                    scale: [1.0, 1.0, 1.0],
                    axis: [0.0, 1.0, 0.0, 0.0],
                }];
                let instbuf = gpu_web::GameBuffer::create(
                    std::mem::size_of_val(&inst) as u32,
                    gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                    "render_web.mesh.surface.instance",
                );
                let Some(instbuf) = instbuf else { return 5 };
                instbuf.write(as_bytes(&inst));
                state.surface_instance_buf = Some(instbuf);
            }
            if let Some(vbuf) = state.surface_vertex_buf.as_ref() {
                vbuf.write(as_bytes(&surface.verts));
            }
            state.surface_snap_key = Some(key);
        }
        let (Some(vbuf), Some(ibuf), Some(instbuf)) = (
            state.surface_vertex_buf.as_ref(),
            state.surface_index_buf.as_ref(),
            state.surface_instance_buf.as_ref(),
        ) else {
            return 0;
        };
        gpu_web::render_mesh(
            &state.target,
            &state.ground_pipeline,
            &state.bind_group,
            vbuf,
            ibuf,
            instbuf,
            state.surface_index_count,
            1,
            0,
        )
    })
}

/// Building walls — baked wall meshes (wall_bake), re-baked and
/// re-uploaded only when the player crosses a chunk; every frame just
/// picks each part's cut-away index range and draws. See RENDER.md
/// slice 4.
pub fn frame_walls(snap: &scene::SceneSnapshot, bt: &crate::buildings::BuildingTemplates) -> u32 {
    let pcx = (snap.player.x / crate::chunk::CHUNK_SIZE).floor() as i32;
    let pcz = (snap.player.z / crate::chunk::CHUNK_SIZE).floor() as i32;
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2;
        };
        if state.walls_chunk_key != Some((pcx, pcz)) {
            // Player crossed a chunk: re-bake the visible set and
            // re-upload. Bakes are static per building (version bumps
            // arrive with wall mutation, later), so this is the only
            // upload path.
            let placed =
                crate::wall_bake::visible_wall_bakes(snap.player, bt, crate::chunk::CHUNK_SIZE, 3);
            let mut gpu = Vec::new();
            for pw in &placed {
                let y = crate::terrain::height(pw.anchor.x, pw.anchor.z);
                for part in &pw.bake.parts {
                    let vertex_buf = gpu_web::GameBuffer::create(
                        std::mem::size_of_val(&part.verts[..]) as u32,
                        gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                        "render_web.mesh.wall.vertex",
                    );
                    let index_buf = gpu_web::GameBuffer::create(
                        std::mem::size_of_val(&part.indices[..]) as u32,
                        gpu_web::usage::INDEX | gpu_web::usage::COPY_DST,
                        "render_web.mesh.wall.index",
                    );
                    let inst = [MeshInstance {
                        pos: [pw.anchor.x, y, pw.anchor.z],
                        color: part.color,
                        scale: [1.0, 1.0, 1.0],
                        axis: [0.0, 1.0, 0.0, 0.0],
                    }];
                    let instance_buf = gpu_web::GameBuffer::create(
                        std::mem::size_of_val(&inst) as u32,
                        gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                        "render_web.mesh.wall.instance",
                    );
                    let (Some(vertex_buf), Some(index_buf), Some(instance_buf)) =
                        (vertex_buf, index_buf, instance_buf)
                    else {
                        return 4;
                    };
                    vertex_buf.write(as_bytes(&part.verts));
                    index_buf.write(as_bytes(&part.indices));
                    instance_buf.write(as_bytes(&inst));
                    gpu.push(WallPartGpu {
                        vertex_buf,
                        index_buf,
                        instance_buf,
                        index_total: part.indices.len() as u32,
                        near_start: part.near_start as u32,
                        near_depths: part.near_depths.clone(),
                        anchor: [pw.anchor.x, pw.anchor.z],
                    });
                }
            }
            state.walls_gpu = gpu;
            state.walls_chunk_key = Some((pcx, pcz));
        }
        for part in &state.walls_gpu {
            let anchor = bevy_math::Vec3::new(part.anchor[0], 0.0, part.anchor[1]);
            let draw_count = if crate::wall_bake::player_inside(anchor, snap) {
                let local_depth =
                    (snap.player.x - part.anchor[0]) + (snap.player.z - part.anchor[1]) + 40.0;
                let visible_near = part.near_depths.partition_point(|d| *d <= local_depth);
                part.near_start + (visible_near as u32) * 3
            } else {
                part.index_total
            };
            if draw_count == 0 {
                continue;
            }
            let r = gpu_web::render_mesh(
                &state.target,
                &state.wall_pipeline,
                &state.bind_group,
                &part.vertex_buf,
                &part.index_buf,
                &part.instance_buf,
                draw_count,
                1,
                0,
            );
            if r != 0 {
                return r;
            }
        }
        0
    })
}

/// UI overlay pass — draws the D-pad + HUD quads on top of everything.
/// Grows the UI instance buffer as the quad count changes (the HUD's
/// settings panel adds quads when open), then runs the load-op pass.
pub fn frame_ui(instances: &[dpad::DpadInstance]) -> u32 {
    if instances.is_empty() {
        return 0;
    }
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2;
        };
        if state.ui_instance_buf.is_none() || instances.len() > state.ui_instance_capacity {
            let new_cap = instances.len().max(16);
            let new_size = (new_cap * std::mem::size_of::<dpad::DpadInstance>()) as u32;
            state.ui_instance_buf = gpu_web::GameBuffer::create(
                new_size,
                gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                "render_web.ui.instance",
            );
            state.ui_instance_capacity = new_cap;
        }
        let Some(ui_buf) = state.ui_instance_buf.as_ref() else {
            return 3;
        };
        ui_buf.write(as_bytes(instances));
        gpu_web::render_ui_overlay(
            &state.target,
            &state.ui_pipeline,
            &state.bind_group,
            ui_buf,
            instances.len() as u32,
        )
    })
}

/// Called from lib.rs's _frame to compose the render step: opaque world
/// first, then translucent glass panes, then the ghost pass (cut-away
/// walls + roof at low alpha), then the D-pad + HUD overlay.
pub fn frame_from_app(app: &mut bevy_app::App) -> u32 {
    let snap = scene::snapshot_scene(app);
    let mut instances = scene::snapshot_to_instances(&snap);
    let mut mesh_trees = crate::tree_emit::snapshot_to_mesh_instances_with_wood(&snap);
    let mut glass = scene::snapshot_to_glass_instances(&snap);
    let mut ghost = scene::snapshot_to_ghost_instances(&snap);
    // Sit the browser world on the terrain — the same choke point as the
    // native path (lib.rs).
    scene::drape(&mut instances);
    scene::drape(&mut glass);
    scene::drape(&mut ghost);
    mesh_trees.drape();
    let camera = SceneCamera::follow(
        [snap.player.x, snap.player.y, snap.player.z],
        crate::room::FLOOR_HALF,
    );
    // Elapsed seconds for leaf-wind sway — synthetic ticks (no bevy_time,
    // same model as the campfire flicker). Advances every frame, so the
    // browser canopy ripples continuously.
    let time = app
        .world()
        .get_resource::<crate::FrameCount>()
        .map(|f| f.0)
        .unwrap_or(0) as f32
        * crate::campfire::TICK_SECONDS;
    let world_result = frame(&camera, &instances, time);
    if world_result != 0 {
        return world_result;
    }
    // Mesh pass runs after the opaque cube pass (which cleared colour +
    // depth) and before glass/ghost. Its LoadOp::Load reads the depth
    // buffer already populated by cubes, so tree trunks and canopy
    // elements are correctly occluded by buildings in front of them.
    // Solid ground (with the world-anchored grid in its shader), then
    // trees — all depth-tested over the cleared cube pass.
    let surface_result = frame_surface(snap.player.x, snap.player.z);
    if surface_result != 0 {
        return surface_result;
    }
    if let Some(bt) = app.world().get_resource::<crate::buildings::BuildingTemplates>() {
        let walls_result = frame_walls(&snap, bt);
        if walls_result != 0 {
            return walls_result;
        }
    }
    let mesh_result = frame_mesh(&mesh_trees.wood_by_species, &mesh_trees.canopy_elements);
    if mesh_result != 0 {
        return mesh_result;
    }
    let glass_result = frame_glass(&glass);
    if glass_result != 0 {
        return glass_result;
    }
    let ghost_result = frame_ghost(&ghost);
    if ghost_result != 0 {
        return ghost_result;
    }
    // D-pad, HUD quads, build watermark, NPC-bump "!" — all one UI
    // pass. The watermark is the running binary drawing its own commit.
    let mut ui: Vec<dpad::DpadInstance> = dpad::current_instances().to_vec();
    ui.extend(hud::current_instances());
    ui.extend(crate::watermark::watermark_quads(gpu_web::viewport_size()));
    ui.extend(crate::bang::current_instances());
    ui.extend(crate::tune_hud::current_instances());
    frame_ui(&ui)
}
