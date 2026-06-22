//! Tile pass: one instanced quad per visible tile, fragment shader
//! reads `u_tile_palette[v_tile_kind]` for color and applies an
//! elevation-derived shading multiplier + day-brightness.

use js_sys::Uint16Array;
use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlUniformLocation,
    WebGlVertexArrayObject,
};

use crate::teranos::TileKind;

use super::helpers::{compile_program, create_buffer_with_data, get_uniform};

pub(super) struct TileProgram {
    pub(super) program: WebGlProgram,
    pub(super) vao: WebGlVertexArrayObject,
    pub(super) instance_buffer: WebGlBuffer,
    pub(super) u_camera_px: WebGlUniformLocation,
    pub(super) u_canvas_px: WebGlUniformLocation,
    pub(super) u_world_px_per_tile: WebGlUniformLocation,
    pub(super) u_zoom: WebGlUniformLocation,
    pub(super) u_view_dim: WebGlUniformLocation,
    pub(super) u_day_brightness: WebGlUniformLocation,
    pub(super) u_tile_palette: WebGlUniformLocation,
}

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

pub(super) fn build_tile_program(gl: &Gl) -> Result<TileProgram, JsValue> {
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

pub(super) fn tile_palette_floats() -> [f32; 5 * 3] {
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
