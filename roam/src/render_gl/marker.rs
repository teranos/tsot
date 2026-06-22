//! Marker pass: solid-color squares.
//!
//! Per-instance attributes give world-pixel position, color, and
//! world-pixel size. Same camera math as the tile and flower shaders
//! so everything lines up. Used for the player marker + remote peers.

use js_sys::Uint16Array;
use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlUniformLocation,
    WebGlVertexArrayObject,
};

use super::helpers::{compile_program, create_buffer_with_data, get_uniform};

pub(super) struct MarkerProgram {
    pub(super) program: WebGlProgram,
    pub(super) vao: WebGlVertexArrayObject,
    pub(super) instance_buffer: WebGlBuffer,
    pub(super) u_camera_px: WebGlUniformLocation,
    pub(super) u_canvas_px: WebGlUniformLocation,
    pub(super) u_zoom: WebGlUniformLocation,
}

impl MarkerProgram {
    pub(super) const INSTANCE_STRIDE_FLOATS: usize = 6;
}

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

pub(super) fn build_marker_program(gl: &Gl) -> Result<MarkerProgram, JsValue> {
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
