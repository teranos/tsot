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
//!   S4b — tile renderer via instanced quads
//!   S4c (this) — procedural flower renderer
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
use crate::teranos::{CoreEdge, FlowerColor, FlowerCore, TileKind};
use crate::viewport::{viewport_ptr, VIEWPORT_HEADER_SIZE, VIEWPORT_TILE_SIZE};

thread_local! {
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
    /// Peer positions + sources, refilled by the JS bridge each frame
    /// via `set_peers`. Storage layout matches the wire format: 3
    /// floats per peer = (world_x, world_y, source). `source` is 0.0
    /// for libp2p, 1.0 for BroadcastChannel; Rust picks the color.
    static PEER_STATE: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Source-of-peer-data tag, matching the `f32` source field passed
/// through `set_peers`. Color picked from this in the marker pass.
const PEER_SOURCE_LIBP2P: f32 = 0.0;
const PEER_SOURCE_BROADCASTCHANNEL: f32 = 1.0;

/// Color triplets — RGB in 0..1. Single source of truth for marker
/// colors lives in Rust (consistent with the tile/flower palette).
const PLAYER_MARKER_RGB: [f32; 3] = [0.4, 0.8, 1.0]; // #6cf
const PEER_LIBP2P_RGB: [f32; 3] = [1.0, 0.4, 0.66]; // #f6a
const PEER_BROADCAST_RGB: [f32; 3] = [1.0, 0.66, 0.4]; // #fa6

/// Called by the JS bridge before each `render_frame` to publish the
/// current peer list. The slice is `[x0, y0, src0, x1, y1, src1, ...]`.
/// Anything not on this list disappears from the next frame's draw.
pub fn set_peers(packed: &[f32]) {
    if packed.len() % 3 != 0 {
        emit_error(
            Severity::Warn,
            "roam::render_gl::set_peers",
            "peer array length not a multiple of 3",
            format!("got {} floats; expected groups of (x, y, source)", packed.len()),
        );
        return;
    }
    PEER_STATE.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.extend_from_slice(packed);
    });
}

pub struct Renderer {
    gl: Gl,
    canvas: HtmlCanvasElement,
    tile_prog: TileProgram,
    flower_prog: FlowerProgram,
    marker_prog: MarkerProgram,
    /// Per-frame scratch buffer for tile instance attributes
    /// (tile_kind, elev_offset). Two floats per visible tile.
    tile_instance_scratch: Vec<f32>,
    /// Per-frame scratch buffer for flower instance attributes
    /// (world_tile.xy, petal_center.rgb, petal_edge.rgb,
    /// core_center.rgb, core_edge.rgb, petal_count). 15 floats per
    /// visible flower; matches `FlowerProgram::INSTANCE_STRIDE_FLOATS`.
    flower_instance_scratch: Vec<f32>,
    /// Marker positions populated by `roam_set_peers` from JS and
    /// drained each frame in the marker pass. Storage is
    /// (world_x, world_y, r, g, b, size_world_px) per marker, matching
    /// `MarkerProgram::INSTANCE_STRIDE_FLOATS`.
    marker_instance_scratch: Vec<f32>,
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

struct FlowerProgram {
    program: WebGlProgram,
    vao: WebGlVertexArrayObject,
    instance_buffer: WebGlBuffer,
    u_camera_px: WebGlUniformLocation,
    u_canvas_px: WebGlUniformLocation,
    u_world_px_per_tile: WebGlUniformLocation,
    u_zoom: WebGlUniformLocation,
    u_day_brightness: WebGlUniformLocation,
}

impl FlowerProgram {
    const INSTANCE_STRIDE_FLOATS: usize = 15;
}

struct MarkerProgram {
    program: WebGlProgram,
    vao: WebGlVertexArrayObject,
    instance_buffer: WebGlBuffer,
    u_camera_px: WebGlUniformLocation,
    u_canvas_px: WebGlUniformLocation,
    u_zoom: WebGlUniformLocation,
}

impl MarkerProgram {
    const INSTANCE_STRIDE_FLOATS: usize = 6;
}

// ----- tile shader sources -----

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

    vec2 world_frag_px =
        (vec2(world_tx, world_ty) + a_unit) * u_world_px_per_tile;
    vec2 frag_screen_px =
        (world_frag_px - u_camera_px) * u_zoom + u_canvas_px * 0.5;

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
        // Air: sky background color.
        out_color = vec4(0.0627, 0.0627, 0.0745, 1.0);
        return;
    }
    vec3 base = u_tile_palette[v_tile_kind];
    float b = clamp(1.0 + float(v_elev_offset) * 0.04, 0.4, 1.4) * u_day_brightness;
    out_color = vec4(base * b, 1.0);
}
"#;

// ----- flower shader sources -----
//
// Geometry constants are encoded in the fragment shader as fractions of
// a tile. Petals + core sit inside [0, 1] × [0, 1] tile-local space,
// centered on (0.5, 0.5). Same numbers the canvas2D path used so the
// visual matches: petal radius 0.15, petal distance 0.18, core radius
// 0.10, gradient origin shifted inward by 0.7 of petal radius with
// reach 1.7 of petal radius.
//
// The quad is the full tile (0..1). Pixels outside the flower discs
// `discard` so the tile underneath shows through.

const FLOWER_VS: &str = r#"#version 300 es
precision highp float;

// Unit quad corner in tile-local coords (0..1 × 0..1).
layout(location = 0) in vec2 a_unit;
// Per-instance attributes.
layout(location = 1) in vec2 a_world_tile;
layout(location = 2) in vec3 a_petal_center_rgb;
layout(location = 3) in vec3 a_petal_edge_rgb;
layout(location = 4) in vec3 a_core_center_rgb;
layout(location = 5) in vec3 a_core_edge_rgb;
layout(location = 6) in float a_petal_count;

uniform vec2 u_camera_px;
uniform vec2 u_canvas_px;
uniform float u_world_px_per_tile;
uniform float u_zoom;

out vec2 v_tile_local;
flat out vec3 v_petal_center_rgb;
flat out vec3 v_petal_edge_rgb;
flat out vec3 v_core_center_rgb;
flat out vec3 v_core_edge_rgb;
flat out int v_petal_count;

void main() {
    vec2 world_frag_px =
        (a_world_tile + a_unit) * u_world_px_per_tile;
    vec2 frag_screen_px =
        (world_frag_px - u_camera_px) * u_zoom + u_canvas_px * 0.5;

    vec2 clip = vec2(
        (frag_screen_px.x / u_canvas_px.x) * 2.0 - 1.0,
        1.0 - (frag_screen_px.y / u_canvas_px.y) * 2.0
    );
    gl_Position = vec4(clip, 0.0, 1.0);

    v_tile_local = a_unit;
    v_petal_center_rgb = a_petal_center_rgb;
    v_petal_edge_rgb = a_petal_edge_rgb;
    v_core_center_rgb = a_core_center_rgb;
    v_core_edge_rgb = a_core_edge_rgb;
    v_petal_count = int(a_petal_count + 0.5);
}
"#;

const FLOWER_FS: &str = r#"#version 300 es
precision highp float;

in vec2 v_tile_local;
flat in vec3 v_petal_center_rgb;
flat in vec3 v_petal_edge_rgb;
flat in vec3 v_core_center_rgb;
flat in vec3 v_core_edge_rgb;
flat in int v_petal_count;

uniform float u_day_brightness;

out vec4 out_color;

const float PI = 3.14159265358979;
const float PETAL_R = 0.15;
const float PETAL_DIST = 0.18;
const float CORE_R = 0.10;
const float PETAL_GRAD_SHIFT = 0.7;
const float PETAL_GRAD_REACH = 1.7;
const float AA_SOFTNESS = 0.01;

void main() {
    vec2 frag = v_tile_local - vec2(0.5);
    float dist = length(frag);

    // Core: radial gradient from core_center → core_edge.
    if (dist < CORE_R + AA_SOFTNESS) {
        float t = clamp(dist / CORE_R, 0.0, 1.0);
        vec3 col = mix(v_core_center_rgb, v_core_edge_rgb, t);
        float alpha = 1.0 - smoothstep(CORE_R - AA_SOFTNESS, CORE_R + AA_SOFTNESS, dist);
        if (alpha > 0.0) {
            out_color = vec4(col * u_day_brightness, alpha);
            return;
        }
    }

    int n = v_petal_count;
    if (n < 1) {
        discard;
    }
    float angle = atan(frag.y, frag.x);
    float angle_step = 2.0 * PI / float(n);
    float start_angle = -PI / 2.0;
    float relative = angle - start_angle;
    // Snap to nearest petal index, accounting for wrap-around.
    int k = int(floor(relative / angle_step + 0.5));
    float petal_angle = start_angle + float(k) * angle_step;
    vec2 petal_dir = vec2(cos(petal_angle), sin(petal_angle));
    vec2 petal_center = petal_dir * PETAL_DIST;
    vec2 frag_to_petal = frag - petal_center;
    float petal_dist = length(frag_to_petal);

    if (petal_dist < PETAL_R + AA_SOFTNESS) {
        // Gradient origin shifted inward toward tile center so the
        // bright color sits on the core-facing side and the edge
        // color is on the outer rim.
        vec2 grad_origin = petal_center - petal_dir * (PETAL_R * PETAL_GRAD_SHIFT);
        float gd = length(frag - grad_origin);
        float t = clamp(gd / (PETAL_R * PETAL_GRAD_REACH), 0.0, 1.0);
        vec3 col = mix(v_petal_center_rgb, v_petal_edge_rgb, t);
        float alpha = 1.0 - smoothstep(PETAL_R - AA_SOFTNESS, PETAL_R + AA_SOFTNESS, petal_dist);
        if (alpha > 0.0) {
            out_color = vec4(col * u_day_brightness, alpha);
            return;
        }
    }

    discard;
}
"#;

// ----- marker shader sources -----
//
// Solid-color squares. Per-instance attributes give world-pixel
// position, color, and world-pixel size. Same camera math as the tile
// and flower shaders so everything lines up.

const MARKER_VS: &str = r#"#version 300 es
precision highp float;

// Unit quad corner in 0..1 space — centered to ±0.5 inside the shader.
layout(location = 0) in vec2 a_unit;
layout(location = 1) in vec2 a_world_pos;
layout(location = 2) in vec3 a_color;
layout(location = 3) in float a_size_world_px;

uniform vec2 u_camera_px;
uniform vec2 u_canvas_px;
uniform float u_zoom;

flat out vec3 v_color;

void main() {
    // Center the unit quad and scale to the requested world size.
    vec2 local = (a_unit - vec2(0.5)) * a_size_world_px;
    vec2 world_frag_px = a_world_pos + local;
    vec2 frag_screen_px =
        (world_frag_px - u_camera_px) * u_zoom + u_canvas_px * 0.5;
    vec2 clip = vec2(
        (frag_screen_px.x / u_canvas_px.x) * 2.0 - 1.0,
        1.0 - (frag_screen_px.y / u_canvas_px.y) * 2.0
    );
    gl_Position = vec4(clip, 0.0, 1.0);
    v_color = a_color;
}
"#;

const MARKER_FS: &str = r#"#version 300 es
precision highp float;

flat in vec3 v_color;

out vec4 out_color;

void main() {
    out_color = vec4(v_color, 1.0);
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

    // Alpha blending for soft flower edges.
    gl.enable(Gl::BLEND);
    gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);

    let tile_prog = build_tile_program(&gl)?;
    let flower_prog = build_flower_program(&gl)?;
    let marker_prog = build_marker_program(&gl)?;

    RENDERER.with(|cell| {
        *cell.borrow_mut() = Some(Renderer {
            gl,
            canvas,
            tile_prog,
            flower_prog,
            marker_prog,
            tile_instance_scratch: Vec::new(),
            flower_instance_scratch: Vec::new(),
            marker_instance_scratch: Vec::new(),
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

    let vao = gl
        .create_vertex_array()
        .ok_or_else(|| JsValue::from_str("gl.createVertexArray returned null"))?;
    gl.bind_vertex_array(Some(&vao));

    // Unit quad mesh.
    let unit_quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let unit_buffer = create_buffer_with_data(gl, &unit_quad)?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&unit_buffer));
    gl.vertex_attrib_pointer_with_i32(0, 2, Gl::FLOAT, false, 0, 0);
    gl.enable_vertex_attrib_array(0);

    // Index buffer (two tris).
    let indices: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let idx_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (idx) returned null"))?;
    gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&idx_buffer));
    unsafe {
        let view = Uint16Array::view(&indices);
        gl.buffer_data_with_array_buffer_view(Gl::ELEMENT_ARRAY_BUFFER, &view, Gl::STATIC_DRAW);
    }

    // Per-instance buffer: 2 floats per tile.
    let instance_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (tile instance) returned null"))?;
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

fn build_flower_program(gl: &Gl) -> Result<FlowerProgram, JsValue> {
    let program = compile_program(gl, FLOWER_VS, FLOWER_FS, "flower")?;

    let u_camera_px = get_uniform(gl, &program, "u_camera_px")?;
    let u_canvas_px = get_uniform(gl, &program, "u_canvas_px")?;
    let u_world_px_per_tile = get_uniform(gl, &program, "u_world_px_per_tile")?;
    let u_zoom = get_uniform(gl, &program, "u_zoom")?;
    let u_day_brightness = get_uniform(gl, &program, "u_day_brightness")?;

    let vao = gl
        .create_vertex_array()
        .ok_or_else(|| JsValue::from_str("gl.createVertexArray returned null"))?;
    gl.bind_vertex_array(Some(&vao));

    // Unit quad mesh + index buffer, same as tiles. Each program has
    // its own VAO; the shared buffer objects could be reused, but
    // re-creating them is trivial and keeps the VAO self-contained.
    let unit_quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let unit_buffer = create_buffer_with_data(gl, &unit_quad)?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&unit_buffer));
    gl.vertex_attrib_pointer_with_i32(0, 2, Gl::FLOAT, false, 0, 0);
    gl.enable_vertex_attrib_array(0);

    let indices: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let idx_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (flower idx) returned null"))?;
    gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&idx_buffer));
    unsafe {
        let view = Uint16Array::view(&indices);
        gl.buffer_data_with_array_buffer_view(Gl::ELEMENT_ARRAY_BUFFER, &view, Gl::STATIC_DRAW);
    }

    // Per-instance buffer. 15 floats per flower; six attribute slots:
    //   loc 1: vec2  a_world_tile           offset 0,  size 8
    //   loc 2: vec3  a_petal_center_rgb     offset 8,  size 12
    //   loc 3: vec3  a_petal_edge_rgb       offset 20, size 12
    //   loc 4: vec3  a_core_center_rgb      offset 32, size 12
    //   loc 5: vec3  a_core_edge_rgb        offset 44, size 12
    //   loc 6: float a_petal_count          offset 56, size 4
    // Stride = 60 bytes.
    let instance_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (flower instance) returned null"))?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&instance_buffer));
    let stride = (FlowerProgram::INSTANCE_STRIDE_FLOATS * 4) as i32;
    gl.vertex_attrib_pointer_with_i32(1, 2, Gl::FLOAT, false, stride, 0);
    gl.vertex_attrib_pointer_with_i32(2, 3, Gl::FLOAT, false, stride, 8);
    gl.vertex_attrib_pointer_with_i32(3, 3, Gl::FLOAT, false, stride, 20);
    gl.vertex_attrib_pointer_with_i32(4, 3, Gl::FLOAT, false, stride, 32);
    gl.vertex_attrib_pointer_with_i32(5, 3, Gl::FLOAT, false, stride, 44);
    gl.vertex_attrib_pointer_with_i32(6, 1, Gl::FLOAT, false, stride, 56);
    for loc in 1..=6 {
        gl.enable_vertex_attrib_array(loc);
        gl.vertex_attrib_divisor(loc, 1);
    }

    gl.bind_vertex_array(None);

    Ok(FlowerProgram {
        program,
        vao,
        instance_buffer,
        u_camera_px,
        u_canvas_px,
        u_world_px_per_tile,
        u_zoom,
        u_day_brightness,
    })
}

fn build_marker_program(gl: &Gl) -> Result<MarkerProgram, JsValue> {
    let program = compile_program(gl, MARKER_VS, MARKER_FS, "marker")?;

    let u_camera_px = get_uniform(gl, &program, "u_camera_px")?;
    let u_canvas_px = get_uniform(gl, &program, "u_canvas_px")?;
    let u_zoom = get_uniform(gl, &program, "u_zoom")?;

    let vao = gl
        .create_vertex_array()
        .ok_or_else(|| JsValue::from_str("gl.createVertexArray returned null"))?;
    gl.bind_vertex_array(Some(&vao));

    let unit_quad: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let unit_buffer = create_buffer_with_data(gl, &unit_quad)?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&unit_buffer));
    gl.vertex_attrib_pointer_with_i32(0, 2, Gl::FLOAT, false, 0, 0);
    gl.enable_vertex_attrib_array(0);

    let indices: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let idx_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (marker idx) returned null"))?;
    gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&idx_buffer));
    unsafe {
        let view = Uint16Array::view(&indices);
        gl.buffer_data_with_array_buffer_view(Gl::ELEMENT_ARRAY_BUFFER, &view, Gl::STATIC_DRAW);
    }

    // Per-instance buffer: 6 floats per marker. Layout matches
    // `MarkerProgram::INSTANCE_STRIDE_FLOATS`.
    //   loc 1: vec2  a_world_pos          offset 0,  size 8
    //   loc 2: vec3  a_color               offset 8,  size 12
    //   loc 3: float a_size_world_px       offset 20, size 4
    // Stride = 24 bytes.
    let instance_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (marker instance) returned null"))?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&instance_buffer));
    let stride = (MarkerProgram::INSTANCE_STRIDE_FLOATS * 4) as i32;
    gl.vertex_attrib_pointer_with_i32(1, 2, Gl::FLOAT, false, stride, 0);
    gl.vertex_attrib_pointer_with_i32(2, 3, Gl::FLOAT, false, stride, 8);
    gl.vertex_attrib_pointer_with_i32(3, 1, Gl::FLOAT, false, stride, 20);
    for loc in 1..=3 {
        gl.enable_vertex_attrib_array(loc);
        gl.vertex_attrib_divisor(loc, 1);
    }

    gl.bind_vertex_array(None);

    Ok(MarkerProgram {
        program,
        vao,
        instance_buffer,
        u_camera_px,
        u_canvas_px,
        u_zoom,
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
            info,
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
            info,
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

        if renderer.canvas.width() != canvas_w {
            renderer.canvas.set_width(canvas_w);
        }
        if renderer.canvas.height() != canvas_h {
            renderer.canvas.set_height(canvas_h);
        }
        renderer.gl.viewport(0, 0, canvas_w as i32, canvas_h as i32);

        // Read the viewport header out of wasm memory.
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
            world_px_per_tile = read_u32_le(ptr, 20) as f32;
        }

        // Build tile instance attributes + collect flower instances in
        // a single pass over the viewport buffer.
        let tile_count = (view_w * view_h) as usize;
        let half_w = view_w as i32 / 2;
        let half_h = view_h as i32 / 2;
        let tile_scratch = &mut renderer.tile_instance_scratch;
        tile_scratch.clear();
        tile_scratch.reserve(tile_count * 2);
        let flower_scratch = &mut renderer.flower_instance_scratch;
        flower_scratch.clear();
        unsafe {
            let ptr = viewport_ptr() as *const u8;
            for i in 0..tile_count {
                let off = VIEWPORT_HEADER_SIZE + i * VIEWPORT_TILE_SIZE;
                let tile_kind = *ptr.add(off) as f32;
                let elev_offset = *ptr.add(off + 1) as i8 as f32;
                tile_scratch.push(tile_kind);
                tile_scratch.push(elev_offset);

                let has_flower = *ptr.add(off + 2);
                if has_flower == 1 {
                    let petal_center = *ptr.add(off + 3);
                    let petal_edge = *ptr.add(off + 4);
                    let core_center = *ptr.add(off + 5);
                    let core_edge_kind = *ptr.add(off + 6);
                    let petal_count = *ptr.add(off + 7);

                    let vx = (i as i32) % (view_w as i32);
                    let vy = (i as i32) / (view_w as i32);
                    let world_tx = (center_tx + vx - half_w) as f32;
                    let world_ty = (center_ty + vy - half_h) as f32;

                    let pc_rgb = flower_color_rgb(petal_center);
                    let pe_rgb = flower_color_rgb(petal_edge);
                    let cc_rgb = flower_core_rgb(core_center);
                    let ce_rgb = match core_edge_from_u8(core_edge_kind) {
                        Some(CoreEdge::White) => [1.0, 1.0, 1.0],
                        Some(CoreEdge::MatchPetalCenter) => pc_rgb,
                        Some(CoreEdge::MatchPetalEdge) => pe_rgb,
                        None => [1.0, 1.0, 1.0],
                    };

                    flower_scratch.push(world_tx);
                    flower_scratch.push(world_ty);
                    flower_scratch.extend_from_slice(&pc_rgb);
                    flower_scratch.extend_from_slice(&pe_rgb);
                    flower_scratch.extend_from_slice(&cc_rgb);
                    flower_scratch.extend_from_slice(&ce_rgb);
                    flower_scratch.push(petal_count as f32);
                }
            }
        }

        let gl = &renderer.gl;

        // ----- clear + tile pass -----
        gl.clear_color(0.0627, 0.0627, 0.0745, 1.0);
        gl.clear(Gl::COLOR_BUFFER_BIT);

        let tile_prog = &renderer.tile_prog;
        gl.bind_vertex_array(Some(&tile_prog.vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&tile_prog.instance_buffer));
        unsafe {
            let view = Float32Array::view(tile_scratch);
            gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
        }
        gl.use_program(Some(&tile_prog.program));
        gl.uniform2f(Some(&tile_prog.u_camera_px), player_x_px, player_y_px);
        gl.uniform2f(Some(&tile_prog.u_canvas_px), canvas_w as f32, canvas_h as f32);
        gl.uniform1f(Some(&tile_prog.u_world_px_per_tile), world_px_per_tile);
        gl.uniform1f(Some(&tile_prog.u_zoom), zoom);
        gl.uniform4i(
            Some(&tile_prog.u_view_dim),
            view_w as i32,
            view_h as i32,
            center_tx,
            center_ty,
        );
        gl.uniform1f(Some(&tile_prog.u_day_brightness), day_brightness);
        let tile_palette = tile_palette_floats();
        gl.uniform3fv_with_f32_array(Some(&tile_prog.u_tile_palette), &tile_palette);
        gl.draw_elements_instanced_with_i32(
            Gl::TRIANGLES,
            6,
            Gl::UNSIGNED_SHORT,
            0,
            tile_count as i32,
        );

        // ----- flower pass -----
        let flower_count =
            flower_scratch.len() / FlowerProgram::INSTANCE_STRIDE_FLOATS;
        if flower_count > 0 {
            let flower_prog = &renderer.flower_prog;
            gl.bind_vertex_array(Some(&flower_prog.vao));
            gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&flower_prog.instance_buffer));
            unsafe {
                let view = Float32Array::view(flower_scratch);
                gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
            }
            gl.use_program(Some(&flower_prog.program));
            gl.uniform2f(Some(&flower_prog.u_camera_px), player_x_px, player_y_px);
            gl.uniform2f(
                Some(&flower_prog.u_canvas_px),
                canvas_w as f32,
                canvas_h as f32,
            );
            gl.uniform1f(Some(&flower_prog.u_world_px_per_tile), world_px_per_tile);
            gl.uniform1f(Some(&flower_prog.u_zoom), zoom);
            gl.uniform1f(Some(&flower_prog.u_day_brightness), day_brightness);
            gl.draw_elements_instanced_with_i32(
                Gl::TRIANGLES,
                6,
                Gl::UNSIGNED_SHORT,
                0,
                flower_count as i32,
            );
        }

        // ----- marker pass -----
        //
        // Player marker is always first; remote peers follow. Marker
        // size matches the original canvas2D logic: 14 screen px at
        // zoom 1, clamped to [8, 32]. Convert to world pixels so
        // we can place the quad in world-space and reuse the zoom
        // uniform path.
        let marker_screen_size = (14.0_f32 * zoom).clamp(8.0, 32.0);
        let marker_world_size = marker_screen_size / zoom;
        let marker_scratch = &mut renderer.marker_instance_scratch;
        marker_scratch.clear();

        // Player.
        marker_scratch.push(player_x_px);
        marker_scratch.push(player_y_px);
        marker_scratch.extend_from_slice(&PLAYER_MARKER_RGB);
        marker_scratch.push(marker_world_size);

        // Peers.
        PEER_STATE.with(|cell| {
            let peers = cell.borrow();
            for chunk in peers.chunks_exact(3) {
                let x = chunk[0];
                let y = chunk[1];
                let source = chunk[2];
                marker_scratch.push(x);
                marker_scratch.push(y);
                let rgb = if (source - PEER_SOURCE_LIBP2P).abs() < f32::EPSILON {
                    PEER_LIBP2P_RGB
                } else if (source - PEER_SOURCE_BROADCASTCHANNEL).abs() < f32::EPSILON {
                    PEER_BROADCAST_RGB
                } else {
                    // Unknown source — surface and fall back to magenta.
                    emit_error(
                        Severity::Warn,
                        "roam::render_gl::render_frame",
                        "peer source tag out of range",
                        format!(
                            "expected 0.0 (libp2p) or 1.0 (BroadcastChannel), got {source}"
                        ),
                    );
                    [1.0, 0.0, 1.0]
                };
                marker_scratch.extend_from_slice(&rgb);
                marker_scratch.push(marker_world_size);
            }
        });

        let marker_count =
            marker_scratch.len() / MarkerProgram::INSTANCE_STRIDE_FLOATS;
        if marker_count > 0 {
            let marker_prog = &renderer.marker_prog;
            gl.bind_vertex_array(Some(&marker_prog.vao));
            gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&marker_prog.instance_buffer));
            unsafe {
                let view = Float32Array::view(marker_scratch);
                gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
            }
            gl.use_program(Some(&marker_prog.program));
            gl.uniform2f(Some(&marker_prog.u_camera_px), player_x_px, player_y_px);
            gl.uniform2f(
                Some(&marker_prog.u_canvas_px),
                canvas_w as f32,
                canvas_h as f32,
            );
            gl.uniform1f(Some(&marker_prog.u_zoom), zoom);
            gl.draw_elements_instanced_with_i32(
                Gl::TRIANGLES,
                6,
                Gl::UNSIGNED_SHORT,
                0,
                marker_count as i32,
            );
        }

        // Sacred-error compliance: surface GL errors through the
        // event log instead of leaving them silent in `gl.getError()`.
        let err = gl.get_error();
        if err != Gl::NO_ERROR {
            emit_error(
                Severity::Error,
                "roam::render_gl::render_frame",
                format!("gl.getError = 0x{err:x} after draw"),
                format!(
                    "tile_count={tile_count} flower_count={flower_count} marker_count={marker_count} canvas={canvas_w}x{canvas_h}"
                ),
            );
        }

        gl.bind_vertex_array(None);
        Ok(())
    })
}

// ----- palette helpers -----

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

fn rgb_to_floats(rgb: [u8; 3]) -> [f32; 3] {
    [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0]
}

fn flower_color_rgb(discriminant: u8) -> [f32; 3] {
    let color = match discriminant {
        0 => FlowerColor::Red,
        1 => FlowerColor::Yellow,
        2 => FlowerColor::Blue,
        3 => FlowerColor::Purple,
        4 => FlowerColor::Azure,
        5 => FlowerColor::Pink,
        6 => FlowerColor::Glow,
        // Out-of-range discriminant: surface and fall back to magenta
        // so the caller sees something noticeable.
        _ => {
            emit_error(
                Severity::Warn,
                "roam::render_gl::flower_color_rgb",
                "FlowerColor discriminant out of range",
                format!("got {discriminant}, expected 0..=6"),
            );
            return [1.0, 0.0, 1.0];
        }
    };
    rgb_to_floats(color.rgb())
}

fn flower_core_rgb(discriminant: u8) -> [f32; 3] {
    let core = match discriminant {
        0 => FlowerCore::White,
        1 => FlowerCore::Yellow,
        2 => FlowerCore::Black,
        _ => {
            emit_error(
                Severity::Warn,
                "roam::render_gl::flower_core_rgb",
                "FlowerCore discriminant out of range",
                format!("got {discriminant}, expected 0..=2"),
            );
            return [1.0, 0.0, 1.0];
        }
    };
    rgb_to_floats(core.rgb())
}

fn core_edge_from_u8(v: u8) -> Option<CoreEdge> {
    match v {
        0 => Some(CoreEdge::White),
        1 => Some(CoreEdge::MatchPetalCenter),
        2 => Some(CoreEdge::MatchPetalEdge),
        _ => None,
    }
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
