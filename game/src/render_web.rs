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
    self, GHOST_SHADER_WGSL, GLASS_SHADER_WGSL, GpuVertex, MESH_SHADER_WGSL, MeshInstance,
    SHADER_WGSL, SceneCamera, SceneInstance, UI_SHADER_WGSL, as_bytes, cube_geometry,
};
use crate::tree_mesh::{self, MeshVertex};

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuCamera {
    view_proj: [[f32; 4]; 4],
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
    // Mesh pipeline — indexed draw of shared tree meshes (trunk cone +
    // canopy icosahedron) instanced per tree / per canopy station.
    // Vertex + index buffers uploaded ONCE at init (both meshes are
    // deterministic pure functions of no inputs). The instance buffer
    // grows with the tree count and holds trunks first, canopy elements
    // second — one buffer, two draw calls dispatched with a first_instance
    // offset for the canopy pass.
    mesh_pipeline: gpu_web::GameRenderPipeline,
    trunk_vertex_buf: gpu_web::GameBuffer,
    trunk_index_buf: gpu_web::GameBuffer,
    trunk_index_count: u32,
    canopy_vertex_buf: gpu_web::GameBuffer,
    canopy_index_buf: gpu_web::GameBuffer,
    canopy_index_count: u32,
    mesh_instance_buf: Option<gpu_web::GameBuffer>,
    mesh_instance_capacity: usize,
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

    // Bake the two shared tree geometries. Trunk = tapered cone at
    // (base=1, top=0.6, height=1) — per-tree instances uniform-scale
    // this to their (height × ratio) dims, and the shader's
    // inverse-transpose normal correction absorbs any residual scale
    // asymmetry. Canopy = unit icosahedron, per-station instances
    // scale to element radius.
    let (trunk_verts, trunk_indices) = tree_mesh::trunk_mesh(12, 1.0, 0.6, 1.0);
    let (canopy_verts, canopy_indices) = tree_mesh::leaf_quad_mesh();
    let trunk_vertex_buf = gpu_web::GameBuffer::create(
        std::mem::size_of_val(&trunk_verts[..]) as u32,
        gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
        "render_web.mesh.trunk.vertex",
    );
    let trunk_index_buf = gpu_web::GameBuffer::create(
        std::mem::size_of_val(&trunk_indices[..]) as u32,
        gpu_web::usage::INDEX | gpu_web::usage::COPY_DST,
        "render_web.mesh.trunk.index",
    );
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
    let (
        Some(trunk_vertex_buf),
        Some(trunk_index_buf),
        Some(canopy_vertex_buf),
        Some(canopy_index_buf),
    ) = (trunk_vertex_buf, trunk_index_buf, canopy_vertex_buf, canopy_index_buf)
    else {
        obs::emit("[render_web] init: mesh geometry buffer null");
        return false;
    };
    trunk_vertex_buf.write(as_bytes(&trunk_verts));
    trunk_index_buf.write(as_bytes(&trunk_indices));
    canopy_vertex_buf.write(as_bytes(&canopy_verts));
    canopy_index_buf.write(as_bytes(&canopy_indices));
    let trunk_index_count = trunk_indices.len() as u32;
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
            trunk_vertex_buf,
            trunk_index_buf,
            trunk_index_count,
            canopy_vertex_buf,
            canopy_index_buf,
            canopy_index_count,
            mesh_instance_buf: None,
            mesh_instance_capacity: 0,
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
pub fn frame(camera: &SceneCamera, instances: &[SceneInstance]) -> u32 {
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2; // not initialized
        };

        let gpu_camera = GpuCamera {
            view_proj: camera.view_proj(),
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

/// Mesh pass — one instance buffer packs trunks first, then canopy
/// elements; two `render_mesh` calls dispatch with the right index
/// buffer + first_instance offset so both shapes share the pipeline
/// and the upload. A no-op success when there are no trees in view.
pub fn frame_mesh(trunks: &[MeshInstance], canopy: &[MeshInstance]) -> u32 {
    let total = trunks.len() + canopy.len();
    if total == 0 {
        return 0;
    }
    STATE.with(|c| {
        let mut opt = c.borrow_mut();
        let Some(state) = opt.as_mut() else {
            return 2;
        };
        if state.mesh_instance_buf.is_none() || total > state.mesh_instance_capacity {
            let new_cap = total.max(64);
            let new_size = (new_cap * std::mem::size_of::<MeshInstance>()) as u32;
            state.mesh_instance_buf = gpu_web::GameBuffer::create(
                new_size,
                gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                "render_web.mesh.instance",
            );
            state.mesh_instance_capacity = new_cap;
        }
        let Some(mesh_buf) = state.mesh_instance_buf.as_ref() else {
            return 3;
        };
        // One buffer, two contiguous slices — write in one go.
        let mut packed: Vec<MeshInstance> = Vec::with_capacity(total);
        packed.extend_from_slice(trunks);
        packed.extend_from_slice(canopy);
        mesh_buf.write(as_bytes(&packed));

        if !trunks.is_empty() {
            let r = gpu_web::render_mesh(
                &state.target,
                &state.mesh_pipeline,
                &state.bind_group,
                &state.trunk_vertex_buf,
                &state.trunk_index_buf,
                mesh_buf,
                state.trunk_index_count,
                trunks.len() as u32,
                0,
            );
            if r != 0 {
                return r;
            }
        }
        if !canopy.is_empty() {
            let r = gpu_web::render_mesh(
                &state.target,
                &state.mesh_pipeline,
                &state.bind_group,
                &state.canopy_vertex_buf,
                &state.canopy_index_buf,
                mesh_buf,
                state.canopy_index_count,
                canopy.len() as u32,
                trunks.len() as u32,
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
    let instances = scene::snapshot_to_instances(&snap);
    let mesh_trees = scene::snapshot_to_mesh_instances(&snap);
    let glass = scene::snapshot_to_glass_instances(&snap);
    let ghost = scene::snapshot_to_ghost_instances(&snap);
    let camera = SceneCamera::follow(
        [snap.player.x, snap.player.y, snap.player.z],
        crate::room::FLOOR_HALF,
    );
    let world_result = frame(&camera, &instances);
    if world_result != 0 {
        return world_result;
    }
    // Mesh pass runs after the opaque cube pass (which cleared colour +
    // depth) and before glass/ghost. Its LoadOp::Load reads the depth
    // buffer already populated by cubes, so tree trunks and canopy
    // elements are correctly occluded by buildings in front of them.
    let mesh_result = frame_mesh(&mesh_trees.trunks, &mesh_trees.canopy_elements);
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
    frame_ui(&ui)
}
