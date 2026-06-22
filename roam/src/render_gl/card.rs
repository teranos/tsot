//! Card pass: placeholder render of a card on the ground.
//!
//! A centered upright rectangle on the tile with a thin dark border,
//! filled with the card-derived color from `card_color_rgb(card_seed)`.
//! Visually distinct from flowers (round-ish, multi-color, varied
//! petal counts). Per-instance attributes are
//! `(world_tx, world_ty, r, g, b)`. Stride = 20 bytes.

use js_sys::Uint16Array;
use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlUniformLocation,
    WebGlVertexArrayObject,
};

use super::helpers::{compile_program, create_buffer_with_data, get_uniform};

pub(super) struct CardProgram {
    pub(super) program: WebGlProgram,
    pub(super) vao: WebGlVertexArrayObject,
    pub(super) instance_buffer: WebGlBuffer,
    pub(super) u_camera_px: WebGlUniformLocation,
    pub(super) u_canvas_px: WebGlUniformLocation,
    pub(super) u_world_px_per_tile: WebGlUniformLocation,
    pub(super) u_zoom: WebGlUniformLocation,
    pub(super) u_day_brightness: WebGlUniformLocation,
}

impl CardProgram {
    /// `(world_tx, world_ty, r, g, b)` per instance.
    pub(super) const INSTANCE_STRIDE_FLOATS: usize = 5;
}

const CARD_VS: &str = r#"#version 300 es
precision highp float;

layout(location = 0) in vec2 a_unit;       // unit quad corner [0..1]
layout(location = 1) in vec2 a_world_tile; // per-instance tile coords
layout(location = 2) in vec3 a_color;      // per-instance card color

uniform vec2 u_camera_px;
uniform vec2 u_canvas_px;
uniform float u_world_px_per_tile;
uniform float u_zoom;

out vec2 v_tile_local;
flat out vec3 v_color;

void main() {
    vec2 world_frag_px = (a_world_tile + a_unit) * u_world_px_per_tile;
    vec2 frag_screen_px = (world_frag_px - u_camera_px) * u_zoom + u_canvas_px * 0.5;
    vec2 clip = vec2(
        (frag_screen_px.x / u_canvas_px.x) * 2.0 - 1.0,
        1.0 - (frag_screen_px.y / u_canvas_px.y) * 2.0
    );
    gl_Position = vec4(clip, 0.0, 1.0);
    v_tile_local = a_unit;
    v_color = a_color;
}
"#;

const CARD_FS: &str = r#"#version 300 es
precision highp float;

in vec2 v_tile_local;
flat in vec3 v_color;

uniform float u_day_brightness;

out vec4 out_color;

const float HALF_W = 0.20;
const float HALF_H = 0.30;
const float BORDER = 0.025;

void main() {
    vec2 d = v_tile_local - vec2(0.5);
    if (abs(d.x) > HALF_W || abs(d.y) > HALF_H) {
        discard;
    }
    bool on_border = abs(d.x) > HALF_W - BORDER || abs(d.y) > HALF_H - BORDER;
    vec3 col = on_border ? vec3(0.05) : v_color;
    out_color = vec4(col * u_day_brightness, 1.0);
}
"#;

pub(super) fn build_card_program(gl: &Gl) -> Result<CardProgram, JsValue> {
    let program = compile_program(gl, CARD_VS, CARD_FS, "card")?;

    let u_camera_px = get_uniform(gl, &program, "u_camera_px")?;
    let u_canvas_px = get_uniform(gl, &program, "u_canvas_px")?;
    let u_world_px_per_tile = get_uniform(gl, &program, "u_world_px_per_tile")?;
    let u_zoom = get_uniform(gl, &program, "u_zoom")?;
    let u_day_brightness = get_uniform(gl, &program, "u_day_brightness")?;

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
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (card idx) returned null"))?;
    gl.bind_buffer(Gl::ELEMENT_ARRAY_BUFFER, Some(&idx_buffer));
    unsafe {
        let view = Uint16Array::view(&indices);
        gl.buffer_data_with_array_buffer_view(Gl::ELEMENT_ARRAY_BUFFER, &view, Gl::STATIC_DRAW);
    }

    // Per-instance buffer. 5 floats per card; two attribute slots:
    //   loc 1: vec2 a_world_tile   offset 0, size 8
    //   loc 2: vec3 a_color        offset 8, size 12
    // Stride = 20 bytes.
    let instance_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (card instance) returned null"))?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&instance_buffer));
    let stride = (CardProgram::INSTANCE_STRIDE_FLOATS * 4) as i32;
    gl.vertex_attrib_pointer_with_i32(1, 2, Gl::FLOAT, false, stride, 0);
    gl.vertex_attrib_pointer_with_i32(2, 3, Gl::FLOAT, false, stride, 8);
    for loc in 1..=2 {
        gl.enable_vertex_attrib_array(loc);
        gl.vertex_attrib_divisor(loc, 1);
    }

    gl.bind_vertex_array(None);

    Ok(CardProgram {
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

/// Deterministic placeholder color for a card from its seed (a u32
/// derived from the ccg string id via `Catalog::seed_at_index`). The
/// actual catalog will eventually carry per-card palette / image bytes;
/// until then a hash of the seed gives every card a stable,
/// distinguishable color that follows the card across catalog
/// reorders. `wrapping_mul` + xor-fold keeps the channels uncorrelated.
pub(super) fn card_color_rgb(card_seed: u32) -> [f32; 3] {
    let mut h = (card_seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 32;
    let r = ((h >> 16) & 0xFF) as u8;
    let g = ((h >> 8) & 0xFF) as u8;
    let b = (h & 0xFF) as u8;
    // Lift each channel into the bright half so the card pops against
    // grass/rock rather than rendering near-black for low-byte seeds.
    let lift = |c: u8| 0.5 + (c as f32 / 255.0) * 0.5;
    [lift(r), lift(g), lift(b)]
}
