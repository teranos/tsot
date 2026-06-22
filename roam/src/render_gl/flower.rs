//! Flower pass: procedural per-tile flower render.
//!
//! Geometry constants live in the fragment shader as fractions of a
//! tile. Petals + core sit inside `[0, 1] × [0, 1]` tile-local space,
//! centered on `(0.5, 0.5)`. Pixels outside the flower discs `discard`
//! so the tile underneath shows through.

use js_sys::Uint16Array;
use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlUniformLocation,
    WebGlVertexArrayObject,
};

use crate::error::{emit as emit_error, Severity};
use crate::teranos::{CoreEdge, FlowerColor, FlowerCore};

use super::helpers::{compile_program, create_buffer_with_data, get_uniform, rgb_to_floats};

pub(super) struct FlowerProgram {
    pub(super) program: WebGlProgram,
    pub(super) vao: WebGlVertexArrayObject,
    pub(super) instance_buffer: WebGlBuffer,
    pub(super) u_camera_px: WebGlUniformLocation,
    pub(super) u_canvas_px: WebGlUniformLocation,
    pub(super) u_world_px_per_tile: WebGlUniformLocation,
    pub(super) u_zoom: WebGlUniformLocation,
    pub(super) u_day_brightness: WebGlUniformLocation,
}

impl FlowerProgram {
    pub(super) const INSTANCE_STRIDE_FLOATS: usize = 15;
}

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

pub(super) fn build_flower_program(gl: &Gl) -> Result<FlowerProgram, JsValue> {
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

pub(super) fn flower_color_rgb(discriminant: u8) -> [f32; 3] {
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

pub(super) fn flower_core_rgb(discriminant: u8) -> [f32; 3] {
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

pub(super) fn core_edge_from_u8(v: u8) -> Option<CoreEdge> {
    match v {
        0 => Some(CoreEdge::White),
        1 => Some(CoreEdge::MatchPetalCenter),
        2 => Some(CoreEdge::MatchPetalEdge),
        _ => None,
    }
}
