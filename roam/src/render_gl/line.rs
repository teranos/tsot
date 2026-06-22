//! Line pass: vertex stream of `(world_pos, rgba)`, drawn as `gl.LINES`.
//!
//! Used for the facing arrow and cliff outlines. Per-vertex alpha lets
//! the cliff outlines bleed through with translucency; the facing
//! arrow keeps alpha at 1.0.
//!
//! WebGL2 caps line width at 1px on most drivers — visually thinner
//! than the original canvas2D 2px. If that's not enough contrast, a
//! separate quad-based line pass replaces this; not blocking the slice.

use wasm_bindgen::JsValue;
use web_sys::{
    WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlUniformLocation,
    WebGlVertexArrayObject,
};

use crate::error::{emit as emit_error, Severity};

use super::helpers::{compile_program, get_uniform};

pub(super) struct LineProgram {
    pub(super) program: WebGlProgram,
    pub(super) vao: WebGlVertexArrayObject,
    pub(super) vertex_buffer: WebGlBuffer,
    pub(super) u_camera_px: WebGlUniformLocation,
    pub(super) u_canvas_px: WebGlUniformLocation,
    pub(super) u_zoom: WebGlUniformLocation,
}

impl LineProgram {
    pub(super) const VERTEX_STRIDE_FLOATS: usize = 6;
}

/// Cliff outline: same translucent black the canvas2D path used
/// (`rgba(0, 0, 0, 0.55)`). Drawn between tiles whose elevation
/// delta exceeds the walkable step.
pub(super) const CLIFF_RGBA: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
/// Facing arrow color, matches canvas2D `#cfc` stroke.
pub(super) const FACING_ARROW_RGBA: [f32; 4] = [0.8, 1.0, 0.8, 1.0];
/// Elevation delta beyond which two adjacent tiles are an unwalkable
/// cliff. Mirrors `roam::world::MAX_STEP_UP_DOWN`.
pub(super) const CLIFF_THRESHOLD: f32 = 1.0;

const LINE_VS: &str = r#"#version 300 es
precision highp float;

layout(location = 0) in vec2 a_world_pos;
layout(location = 1) in vec4 a_color;

uniform vec2 u_camera_px;
uniform vec2 u_canvas_px;
uniform float u_zoom;

out vec4 v_color;

void main() {
    vec2 frag_screen_px =
        (a_world_pos - u_camera_px) * u_zoom + u_canvas_px * 0.5;
    vec2 clip = vec2(
        (frag_screen_px.x / u_canvas_px.x) * 2.0 - 1.0,
        1.0 - (frag_screen_px.y / u_canvas_px.y) * 2.0
    );
    gl_Position = vec4(clip, 0.0, 1.0);
    v_color = a_color;
}
"#;

const LINE_FS: &str = r#"#version 300 es
precision highp float;

in vec4 v_color;
out vec4 out_color;

void main() {
    out_color = v_color;
}
"#;

pub(super) fn build_line_program(gl: &Gl) -> Result<LineProgram, JsValue> {
    let program = compile_program(gl, LINE_VS, LINE_FS, "line")?;

    let u_camera_px = get_uniform(gl, &program, "u_camera_px")?;
    let u_canvas_px = get_uniform(gl, &program, "u_canvas_px")?;
    let u_zoom = get_uniform(gl, &program, "u_zoom")?;

    let vao = gl
        .create_vertex_array()
        .ok_or_else(|| JsValue::from_str("gl.createVertexArray returned null"))?;
    gl.bind_vertex_array(Some(&vao));

    let vertex_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("gl.createBuffer (line vertex) returned null"))?;
    gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&vertex_buffer));

    // Per-vertex layout matches `LineProgram::VERTEX_STRIDE_FLOATS`:
    //   loc 0: vec2 a_world_pos  offset 0, size 8
    //   loc 1: vec4 a_color      offset 8, size 16
    // Stride = 24 bytes.
    let stride = (LineProgram::VERTEX_STRIDE_FLOATS * 4) as i32;
    gl.vertex_attrib_pointer_with_i32(0, 2, Gl::FLOAT, false, stride, 0);
    gl.vertex_attrib_pointer_with_i32(1, 4, Gl::FLOAT, false, stride, 8);
    gl.enable_vertex_attrib_array(0);
    gl.enable_vertex_attrib_array(1);

    gl.bind_vertex_array(None);

    Ok(LineProgram {
        program,
        vao,
        vertex_buffer,
        u_camera_px,
        u_canvas_px,
        u_zoom,
    })
}

/// Push two vertices (one line segment) into the per-frame line
/// vertex buffer. Layout matches `LineProgram::VERTEX_STRIDE_FLOATS`:
/// 6 floats per vertex (world.xy + rgba). Two vertices per line.
pub(super) fn push_line(
    buf: &mut Vec<f32>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    rgba: [f32; 4],
) {
    buf.push(x0);
    buf.push(y0);
    buf.extend_from_slice(&rgba);
    buf.push(x1);
    buf.push(y1);
    buf.extend_from_slice(&rgba);
}

/// 8-way facing → unit `(dx, dy)` for the facing arrow. Matches the
/// `Facing` discriminant order in `roam::world::Facing`.
pub(super) fn facing_unit_vec(facing: u8) -> (f32, f32) {
    use core::f32::consts::FRAC_1_SQRT_2 as S;
    match facing {
        0 => (0.0, -1.0),  // N
        1 => (S, -S),      // NE
        2 => (1.0, 0.0),   // E
        3 => (S, S),       // SE
        4 => (0.0, 1.0),   // S
        5 => (-S, S),      // SW
        6 => (-1.0, 0.0),  // W
        7 => (-S, -S),     // NW
        _ => {
            emit_error(
                Severity::Warn,
                "roam::render_gl::facing_unit_vec",
                "facing byte out of range",
                format!("got {facing}, expected 0..=7; defaulting to S"),
            );
            (0.0, 1.0)
        }
    }
}
