//! Shared WebGL2 helpers used by every program in `render_gl`.
//!
//! Visibility: every helper is `pub(super)` so siblings (`tile`,
//! `flower`, `card`, `marker`, `line`) can use them; nothing escapes
//! the `render_gl` module.

use js_sys::Float32Array;
use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext as Gl, WebGlBuffer, WebGlProgram, WebGlShader, WebGlUniformLocation};

use crate::error::{emit as emit_error, Severity};

pub(super) fn compile_program(
    gl: &Gl,
    vs_src: &str,
    fs_src: &str,
    name: &str,
) -> Result<WebGlProgram, JsValue> {
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

pub(super) fn compile_shader(
    gl: &Gl,
    ty: u32,
    src: &str,
    name: &str,
) -> Result<WebGlShader, JsValue> {
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

pub(super) fn get_uniform(
    gl: &Gl,
    program: &WebGlProgram,
    name: &str,
) -> Result<WebGlUniformLocation, JsValue> {
    gl.get_uniform_location(program, name)
        .ok_or_else(|| JsValue::from_str(&format!("uniform '{name}' not found in program")))
}

pub(super) fn create_buffer_with_data(gl: &Gl, data: &[f32]) -> Result<WebGlBuffer, JsValue> {
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

pub(super) fn rgb_to_floats(rgb: [u8; 3]) -> [f32; 3] {
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
    ]
}
