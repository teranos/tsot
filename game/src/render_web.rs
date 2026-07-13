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
    self, GLASS_SHADER_WGSL, GpuVertex, SHADER_WGSL, SceneCamera, SceneInstance, UI_SHADER_WGSL,
    as_bytes, cube_geometry,
};

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
    // UI overlay — pipeline + growable instance buffer for D-pad + HUD.
    ui_pipeline: gpu_web::GameUiPipeline,
    ui_instance_buf: Option<gpu_web::GameBuffer>,
    ui_instance_capacity: usize,
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
            ui_pipeline,
            ui_instance_buf: None,
            ui_instance_capacity: 0,
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
/// first, then the translucent glass panes, then the D-pad + HUD
/// overlay.
pub fn frame_from_app(app: &mut bevy_app::App) -> u32 {
    let snap = scene::snapshot_scene(app);
    let instances = scene::snapshot_to_instances(&snap);
    let glass = scene::snapshot_to_glass_instances(&snap);
    let camera = SceneCamera::follow(
        [snap.player.x, snap.player.y, snap.player.z],
        crate::room::FLOOR_HALF,
    );
    let world_result = frame(&camera, &instances);
    if world_result != 0 {
        return world_result;
    }
    let glass_result = frame_glass(&glass);
    if glass_result != 0 {
        return glass_result;
    }
    // D-pad, HUD quads, build watermark, NPC-bump "!" — all one UI
    // pass. The watermark is the running binary drawing its own commit.
    let mut ui: Vec<dpad::DpadInstance> = dpad::current_instances().to_vec();
    ui.extend(hud::current_instances());
    ui.extend(crate::watermark::watermark_quads(gpu_web::viewport_size()));
    ui.extend(crate::bang::current_instances());
    frame_ui(&ui)
}
