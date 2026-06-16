//! WebGL2 renderer for the world canvas.
//!
//! Rust owns the entire render pipeline — context, shaders, buffers,
//! draw calls. JS hands us the canvas once at init and never touches a
//! pixel after. web_sys' typed WebGL bindings still compile to JS calls
//! under the hood, but the bridge's draw surface in JS is zero (one
//! `roam_render(...)` per frame).
//!
//! Slice progression:
//!   S4a — context wired, single-color clear per frame
//!   S4b (this) — tile renderer via instanced quads
//!   S4c — flower renderer with procedural fragment shader
//!   S4d — markers, cliffs, status text
//!   S4e — bridge canvas2D draws deleted

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use js_sys::{Float32Array, Uint16Array};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlShader,
    WebGlUniformLocation, WebGlVertexArrayObject,
};

use crate::error::{emit as emit_error, Severity};
use crate::teranos::{FlowerColor, FlowerCore, TileKind};
use crate::viewport::{viewport_ptr, VIEWPORT_HEADER_SIZE, VIEWPORT_TILE_SIZE};

thread_local! {
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
}

pub struct Renderer {
    gl: Gl,
    canvas: HtmlCanvasElement,
    tile_prog: TileProgram,
    /// Per-frame scratch buffer for tile instance attributes
    /// (tile_kind, elev_offset). Two floats per visible tile.
    tile_instance_scratch: Vec<f32>,
}

struct TileProgram {
    program: WebGlProgram,
    vao: WebGlVertexArrayObject,
    instance_buffer: WebGlBuffer,
    u_camera_px: WebGlUniformLocation,
    u_canvas_px: WebGlUniformLocation,
    u_world_px_per_tile: WebGlUniformLocation,
    u_zoom: WebGlUniformLocation,
    u_view_dim: WebGlUniformLocation,
    u_day_brightness: WebGlUniformLocation,
    u_tile_palette: WebGlUniformLocation,
}

// ----- shader sources -----

const TILE_VS: &str = r#"#version 300 es
precision highp float;

// Unit quad corner in tile-local coords (0..1 × 0..1).
layout(location = 0) in vec2 a_unit;
// Per-instance: (tile_kind as float, elev_offset as float).
layout(location = 1) in vec2 a_tile;

// Camera: player position in WORLD pixels.
uniform vec2 u_camera_px;
// Canvas size in CSS pixels.
uniform vec2 u_canvas_px;
// PIXELS_PER_TILE — world-pixel size of one tile (zoom-independent).
uniform float u_world_px_per_tile;
// Render zoom factor (screen px per world px).
uniform float u_zoom;
// Viewport dimensions (view_w, view_h, center_tx, center_ty) packed
// as ivec4 — the per-instance tile position is derived from
// gl_InstanceID + this.
uniform ivec4 u_view_dim;

flat out int v_tile_kind;
flat out int v_elev_offset;

void main() {
    int vx = gl_InstanceID % u_view_dim.x;
    int vy = gl_InstanceID / u_view_dim.x;
    int half_w = u_view_dim.x / 2;
    int half_h = u_view_dim.y / 2;
    int world_tx = u_view_dim.z + vx - half_w;
    int world_ty = u_view_dim.w + vy - half_h;

    // Tile's top-left corner in WORLD pixels, plus the unit quad
    // interpolation inside the tile (still WORLD pixels).
    vec2 world_frag_px =
        (vec2(world_tx, world_ty) + a_unit) * u_world_px_per_tile;
    // Project to SCREEN pixels: subtract camera (still in world px),
    // scale by zoom, add half-canvas to center on the player.
    vec2 frag_screen_px =
        (world_frag_px - u_camera_px) * u_zoom + u_canvas_px * 0.5;

    // Convert to clip space: x in [-1, 1], y in [1, -1] (flip y so +y
    // is down on screen).
    vec2 clip = vec2(
        (frag_screen_px.x / u_canvas_px.x) * 2.0 - 1.0,
        1.0 - (frag_screen_px.y / u_canvas_px.y) * 2.0
    );
    gl_Position = vec4(clip, 0.0, 1.0);

    v_tile_kind = int(a_tile.x);
    v_elev_offset = int(a_tile.y);
}
"#;

const TILE_FS: &str = r#"#version 300 es
precision highp float;

flat in int v_tile_kind;
flat in int v_elev_offset;

uniform float u_day_brightness;
// 5 RGB triples: Air, Grass, Rock, ShallowWater, DeepWater.
uniform vec3 u_tile_palette[5];

out vec4 out_color;

void main() {
    if (v_tile_kind == 0) {
        // Air: render the background sky color. Distinct from "discard"
        // so the canvas behind has nothing to bleed through.
        out_color = vec4(0.0627, 0.0627, 0.0745, 1.0);
        return;
    }
    vec3 base = u_tile_palette[v_tile_kind];
    float b = clamp(1.0 + float(v_elev_offset) * 0.04, 0.4, 1.4) * u_day_brightness;
    out_color = vec4(base * b, 1.0);
}
"#;

// ----- init -----

/// Acquire WebGL2 from the canvas, compile shaders, allocate buffers.
/// Caller is the JS bridge at init. If WebGL2 is unavailable, the
/// failure surfaces through the sacred-error log rather than being
/// swallowed.
pub fn init(canvas: HtmlCanvasElement) -> Result<(), JsValue> {
    let ctx = canvas.get_context("webgl2")?.ok_or_else(|| {
        let err = emit_error(
            Severity::Error,
            "roam::render_gl::init",
            "WebGL2 unavailable",
            "canvas.getContext('webgl2') returned null on this browser",
        );
        JsValue::from_str(&err.why)
    })?;
    let gl = ctx.dyn_into::<Gl>().map_err(|_| {
        let err = emit_error(
            Severity::Error,
            "roam::render_gl::init",
            "WebGL2 context downcast failed",
            "getContext('webgl2') returned an object that isn't a WebGl2RenderingContext",
        );
        JsValue::from_str(&err.why)
    })?;

    let tile_prog = build_tile_program(&gl)?;

    RENDERER.with(|cell| {
        *cell.borrow_mut() = Some(Renderer {
            gl,
            canvas,
            tile_prog,
            tile_instance_scratch: Vec::new(),
        });
    });
    Ok(())
}

fn build_tile_program(gl: &Gl) -> Result<TileProgram, JsValue> {
    let program = compile_program(gl, TILE_VS, TILE_FS, "tile")?;

    let u_camera_px = get_uniform(gl, &program, "u_camera_px")?;
    let u_canvas_px = get_uniform(gl, &program, "u_canvas_px")?;
    let u_world_px_per_tile = get_uniform(gl, &program, "u_world_px_per_tile")?;
    let u_zoom = get_uniform(gl, &program, "u_zoom")?;
    let u_view_dim = get_uniform(gl, &program, "u_view_dim")?;
    let u_day_brightness = get_uniform(gl, &program, "u_day_brightness")?;
    let u_tile_palette = get_uniform(gl, &program, "u_tile_palette[0]")?;

    let vao = gl.create_vertex_array().ok_or_else(|| {
        JsValue::from_str("roam::render_gl: gl.createVertexArray returned null")
    })?;
    gl.bind_vertex_array(Some(&vao));

    // Unit quad mesh: 4 verts, 2 triangles, stored once. Vertex
    // attribute 0 = a_unit (the corner of the tile in 0..1 space).
    let unit_quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let unit_buffer = create_buffer_with_data(gl, &unit_quad)?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&unit_buffer));
    gl.vertex_attrib_pointer_with_i32(0, 2, Gl::FLOAT, false, 0, 0);
    gl.enable_vertex_attrib_array(0);

    // Index buffer for the unit quad (two tris).
    let indices: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let idx_buffer = gl.create_buffer().ok_or_else(|| {
        JsValue::from_str("roam::render_gl: gl.createBuffer (idx) returned null")
    })?;
    gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&idx_buffer));
    unsafe {
        let view = Uint16Array::view(&indices);
        gl.buffer_data_with_array_buffer_view(
            Gl::ELEMENT_ARRAY_BUFFER,
            &view,
            Gl::STATIC_DRAW,
        );
    }

    // Per-instance buffer: 2 floats per tile (tile_kind, elev_offset).
    let instance_buffer = gl.create_buffer().ok_or_else(|| {
        JsValue::from_str("roam::render_gl: gl.createBuffer (instance) returned null")
    })?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&instance_buffer));
    gl.vertex_attrib_pointer_with_i32(1, 2, Gl::FLOAT, false, 0, 0);
    gl.enable_vertex_attrib_array(1);
    gl.vertex_attrib_divisor(1, 1);

    gl.bind_vertex_array(None);

    Ok(TileProgram {
        program,
        vao,
        instance_buffer,
        u_camera_px,
        u_canvas_px,
        u_world_px_per_tile,
        u_zoom,
        u_view_dim,
        u_day_brightness,
        u_tile_palette,
    })
}

fn compile_program(gl: &Gl, vs_src: &str, fs_src: &str, name: &str) -> Result<WebGlProgram, JsValue> {
    let vs = compile_shader(gl, Gl::VERTEX_SHADER, vs_src, name)?;
    let fs = compile_shader(gl, Gl::FRAGMENT_SHADER, fs_src, name)?;
    let program = gl
        .create_program()
        .ok_or_else(|| JsValue::from_str("gl.createProgram returned null"))?;
    gl.attach_shader(&program, &vs);
    gl.attach_shader(&program, &fs);
    gl.link_program(&program);
    let ok = gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false);
    if !ok {
        let info = gl.get_program_info_log(&program).unwrap_or_default();
        let err = emit_error(
            Severity::Error,
            "roam::render_gl::compile_program",
            format!("{name} program link failed"),
            info.clone(),
        );
        return Err(JsValue::from_str(&err.why));
    }
    Ok(program)
}

fn compile_shader(gl: &Gl, ty: u32, src: &str, name: &str) -> Result<WebGlShader, JsValue> {
    let shader = gl
        .create_shader(ty)
        .ok_or_else(|| JsValue::from_str("gl.createShader returned null"))?;
    gl.shader_source(&shader, src);
    gl.compile_shader(&shader);
    let ok = gl
        .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false);
    if !ok {
        let info = gl.get_shader_info_log(&shader).unwrap_or_default();
        let kind = if ty == Gl::VERTEX_SHADER { "vs" } else { "fs" };
        let err = emit_error(
            Severity::Error,
            "roam::render_gl::compile_shader",
            format!("{name} {kind} compile failed"),
            info.clone(),
        );
        return Err(JsValue::from_str(&err.why));
    }
    Ok(shader)
}

fn get_uniform(gl: &Gl, program: &WebGlProgram, name: &str) -> Result<WebGlUniformLocation, JsValue> {
    gl.get_uniform_location(program, name)
        .ok_or_else(|| JsValue::from_str(&format!("uniform '{name}' not found in program")))
}

fn create_buffer_with_data(gl: &Gl, data: &[f32]) -> Result<WebGlBuffer, JsValue> {
    let buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer returned null"))?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&buffer));
    unsafe {
        let view = Float32Array::view(data);
        gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STATIC_DRAW);
    }
    Ok(buffer)
}

// ----- frame draw -----

/// Render one frame to the canvas.
///
/// Arguments are everything the renderer needs that isn't in the
/// viewport buffer: the player's float pixel position (for camera
/// centering at sub-tile precision), zoom level, canvas size in CSS
/// pixels, and the current day brightness (`0.0..=1.0`).
///
/// The viewport buffer (`roam_viewport_write` results) must already
/// hold the latest tile data. The renderer reads it directly out of
/// wasm memory via `viewport_ptr()`.
pub fn render_frame(
    player_x_px: f32,
    player_y_px: f32,
    zoom: f32,
    canvas_w: u32,
    canvas_h: u32,
    day_brightness: f32,
) -> Result<(), JsValue> {
    RENDERER.with(|cell| {
        let mut opt = cell.borrow_mut();
        let Some(renderer) = opt.as_mut() else {
            return Err(JsValue::from_str(
                "roam::render_gl: renderer not initialized — call roam_render_init first",
            ));
        };

        // Sync canvas backing-store size with CSS size if the bridge
        // resized the canvas. This avoids stretching during DPR changes
        // or window resizes.
        if renderer.canvas.width() != canvas_w {
            renderer.canvas.set_width(canvas_w);
        }
        if renderer.canvas.height() != canvas_h {
            renderer.canvas.set_height(canvas_h);
        }
        renderer.gl.viewport(0, 0, canvas_w as i32, canvas_h as i32);

        // Read the viewport header out of wasm memory. The pointer is
        // valid as long as we don't reallocate the viewport buffer
        // between `roam_viewport_write` and this call.
        let view_w;
        let view_h;
        let center_tx;
        let center_ty;
        let world_px_per_tile;
        unsafe {
            let ptr = viewport_ptr() as *const u8;
            view_w = read_u32_le(ptr, 0);
            view_h = read_u32_le(ptr, 4);
            center_tx = read_i32_le(ptr, 8);
            center_ty = read_i32_le(ptr, 12);
            // pixels_per_tile in the header (offset 20) is the
            // WORLD-pixel tile size — zoom is applied in the shader.
            world_px_per_tile = read_u32_le(ptr, 20) as f32;
        }

        // Build the instance buffer: per-tile (tile_kind, elev_offset)
        // as two floats. Skip Air tiles (kind == 0) by reordering — we
        // pack only non-Air tiles and pass a smaller instance count.
        // Actually: we keep the gl_InstanceID-to-tile mapping intact
        // by passing every tile and letting the fragment shader paint
        // the sky color for Air. Simpler; the per-fragment cost of
        // painting Air with the sky color is negligible.
        let tile_count = (view_w * view_h) as usize;
        let scratch = &mut renderer.tile_instance_scratch;
        scratch.clear();
        scratch.reserve(tile_count * 2);
        unsafe {
            let ptr = viewport_ptr() as *const u8;
            for i in 0..tile_count {
                let off = VIEWPORT_HEADER_SIZE + i * VIEWPORT_TILE_SIZE;
                let tile_kind = *ptr.add(off) as f32;
                let elev_offset = *ptr.add(off + 1) as i8 as f32;
                scratch.push(tile_kind);
                scratch.push(elev_offset);
            }
        }

        let gl = &renderer.gl;
        let prog = &renderer.tile_prog;

        gl.bind_vertex_array(Some(&prog.vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&prog.instance_buffer));
        unsafe {
            let view = Float32Array::view(scratch);
            gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
        }

        gl.use_program(Some(&prog.program));
        gl.uniform2f(Some(&prog.u_camera_px), player_x_px, player_y_px);
        gl.uniform2f(Some(&prog.u_canvas_px), canvas_w as f32, canvas_h as f32);
        gl.uniform1f(Some(&prog.u_world_px_per_tile), world_px_per_tile);
        gl.uniform1f(Some(&prog.u_zoom), zoom);
        gl.uniform4i(
            Some(&prog.u_view_dim),
            view_w as i32,
            view_h as i32,
            center_tx,
            center_ty,
        );
        gl.uniform1f(Some(&prog.u_day_brightness), day_brightness);

        // Palette: TileKind RGBs as 5 vec3s.
        let palette = tile_palette_floats();
        gl.uniform3fv_with_f32_array(Some(&prog.u_tile_palette), &palette);

        // Clear, then draw.
        gl.clear_color(0.0627, 0.0627, 0.0745, 1.0);
        gl.clear(Gl::COLOR_BUFFER_BIT);

        gl.draw_elements_instanced_with_i32(
            Gl::TRIANGLES,
            6,
            Gl::UNSIGNED_SHORT,
            0,
            tile_count as i32,
        );

        // GL errors are silent by default. Surface them through the
        // sacred-error log so we see "draw call failed" instead of
        // staring at an unchanging canvas wondering why nothing moves.
        let err = gl.get_error();
        if err != Gl::NO_ERROR {
            emit_error(
                Severity::Error,
                "roam::render_gl::render_frame",
                format!("gl.getError = 0x{err:x} after drawElementsInstanced"),
                format!(
                    "view_w={view_w} view_h={view_h} tile_count={tile_count} canvas={canvas_w}x{canvas_h}"
                ),
            );
        }

        gl.bind_vertex_array(None);
        Ok(())
    })
}

fn tile_palette_floats() -> [f32; 5 * 3] {
    let mut out = [0.0_f32; 15];
    let tiles = [
        TileKind::Air,
        TileKind::Grass,
        TileKind::Rock,
        TileKind::ShallowWater,
        TileKind::DeepWater,
    ];
    for (i, tk) in tiles.iter().enumerate() {
        let [r, g, b] = tk.rgb();
        out[i * 3] = r as f32 / 255.0;
        out[i * 3 + 1] = g as f32 / 255.0;
        out[i * 3 + 2] = b as f32 / 255.0;
    }
    out
}

// Suppress dead-code warnings: these enums are read via FFI for the
// color-table FFI, and we mention them here only to anchor that the
// shader palette agrees with `teranos::*::rgb()`.
#[allow(dead_code)]
fn _palette_anchor() {
    let _ = FlowerColor::Red.rgb();
    let _ = FlowerCore::White.rgb();
}

#[inline]
unsafe fn read_u32_le(ptr: *const u8, offset: usize) -> u32 {
    let p = ptr.add(offset) as *const u32;
    p.read_unaligned().to_le()
}

#[inline]
unsafe fn read_i32_le(ptr: *const u8, offset: usize) -> i32 {
    let p = ptr.add(offset) as *const i32;
    p.read_unaligned().to_le()
}
