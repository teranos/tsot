// Wasm-side WebGPU init. Hand-wired env.* imports over a JS shim.
//
// Encapsulated pattern: JS owns the async chain (navigator.gpu →
// requestAdapter → requestDevice). Rust kicks it off with a policy
// argument and polls status. Rust never sees a Promise.

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuStatus {
    Pending = 0,
    Ready = 1,
    Unavailable = 2,
}

impl GpuStatus {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Ready,
            2 => Self::Unavailable,
            _ => Self::Pending,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerPreference {
    Low = 0,
    High = 1,
}

// GPUBufferUsage flags from the WebGPU spec — passed through unchanged
// to createBuffer on the JS side.
pub mod usage {
    pub const MAP_READ: u32 = 0x0001;
    pub const MAP_WRITE: u32 = 0x0002;
    pub const COPY_SRC: u32 = 0x0004;
    pub const COPY_DST: u32 = 0x0008;
    pub const INDEX: u32 = 0x0010;
    pub const VERTEX: u32 = 0x0020;
    pub const UNIFORM: u32 = 0x0040;
    pub const STORAGE: u32 = 0x0080;
    pub const INDIRECT: u32 = 0x0100;
    pub const QUERY_RESOLVE: u32 = 0x0200;
}

// Format discriminants — the JS shim's decoder tables map these to
// WebGPU format strings. Keep the enum small; add values as game
// needs new formats.
pub mod color_format {
    pub const RGBA8UNORM: u32 = 0;
    pub const BGRA8UNORM: u32 = 1;
}

pub mod depth_format {
    pub const DEPTH32FLOAT: u32 = 0;
    pub const DEPTH24PLUS: u32 = 1;
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_gpu_init(power_pref: u32);
    fn game_gpu_status() -> u32;
    fn game_gpu_buffer_create(size: u32, usage: u32, label_ptr: *const u8, label_len: u32) -> u32;
    fn game_gpu_buffer_write(handle: u32, data_ptr: *const u8, data_len: u32);
    fn game_gpu_buffer_destroy(handle: u32);
    fn game_gpu_shader_module_create(src_ptr: *const u8, src_len: u32, label_ptr: *const u8, label_len: u32) -> u32;
    fn game_gpu_bind_group_layout_create_uniform(label_ptr: *const u8, label_len: u32) -> u32;
    fn game_gpu_bind_group_create(layout: u32, buffer: u32, label_ptr: *const u8, label_len: u32) -> u32;
    fn game_gpu_pipeline_layout_create(bg_layout: u32, label_ptr: *const u8, label_len: u32) -> u32;
    fn game_gpu_render_pipeline_create_cube(
        pipeline_layout: u32,
        shader: u32,
        vertex_stride: u32,
        instance_stride: u32,
        color_format: u32,
        depth_format: u32,
        label_ptr: *const u8,
        label_len: u32,
    ) -> u32;
    fn game_gpu_render_target_configure(
        canvas_id_ptr: *const u8, canvas_id_len: u32,
        color_format: u32, depth_format: u32,
    ) -> u32;
    fn game_gpu_render_frame(
        target: u32,
        pipeline: u32,
        bind_group: u32,
        vertex_buf: u32,
        instance_buf: u32,
        vertex_count: u32,
        instance_count: u32,
        clear_r: f32, clear_g: f32, clear_b: f32,
    ) -> u32;
    fn game_gpu_render_pipeline_create_ui(
        pipeline_layout: u32,
        shader: u32,
        instance_stride: u32,
        color_format: u32,
        label_ptr: *const u8,
        label_len: u32,
    ) -> u32;
    fn game_gpu_render_ui_overlay(
        target: u32,
        pipeline: u32,
        bind_group: u32,
        instance_buf: u32,
        instance_count: u32,
    ) -> u32;
    fn game_gpu_render_pipeline_create_glass(
        pipeline_layout: u32,
        shader: u32,
        vertex_stride: u32,
        instance_stride: u32,
        color_format: u32,
        depth_format: u32,
        label_ptr: *const u8,
        label_len: u32,
    ) -> u32;
    fn game_gpu_render_glass(
        target: u32,
        pipeline: u32,
        bind_group: u32,
        vertex_buf: u32,
        instance_buf: u32,
        vertex_count: u32,
        instance_count: u32,
    ) -> u32;
    fn game_touch_state(out_ptr: *mut u8, out_max: u32) -> u32;
    fn game_viewport_size(out_ptr: *mut u8);
}

/// Query the active touch positions from the JS side. Returns up to
/// MAX entries; each pair is (x, y) in NDC (x: -1 left, +1 right;
/// y: -1 bottom, +1 top per WebGPU convention). The list also
/// includes the mouse cursor while a button is held so desktop
/// testing works.
#[cfg(target_arch = "wasm32")]
pub fn touches() -> Vec<[f32; 2]> {
    const MAX: usize = 8;
    let mut buf: [f32; MAX * 2] = [0.0; MAX * 2];
    let n = unsafe { game_touch_state(buf.as_mut_ptr() as *mut u8, MAX as u32) } as usize;
    let n = n.min(MAX);
    (0..n).map(|i| [buf[i * 2], buf[i * 2 + 1]]).collect()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn touches() -> Vec<[f32; 2]> {
    Vec::new()
}

/// Viewport (client) pixel dimensions from the JS canvas. Used to
/// derive the aspect ratio so UI quads render square regardless of
/// portrait/landscape.
#[cfg(target_arch = "wasm32")]
pub fn viewport_size() -> (u32, u32) {
    let mut out: [u32; 2] = [0; 2];
    unsafe { game_viewport_size(out.as_mut_ptr() as *mut u8) };
    (out[0], out[1])
}

#[cfg(not(target_arch = "wasm32"))]
pub fn viewport_size() -> (u32, u32) {
    (1920, 1080)
}

#[cfg(target_arch = "wasm32")]
pub fn init(pref: PowerPreference) {
    unsafe { game_gpu_init(pref as u32) }
}

#[cfg(target_arch = "wasm32")]
pub fn status() -> GpuStatus {
    GpuStatus::from_u32(unsafe { game_gpu_status() })
}

/// Handle-wrapped GPUBuffer. Drop calls the JS-side destroy — the
/// axiom's whole point: buffer lifetime is Rust-controlled and
/// greppable, never left to a Rust-Drop-vs-JS-destroy mismatch.
#[cfg(target_arch = "wasm32")]
pub struct GameBuffer {
    handle: u32,
}

#[cfg(target_arch = "wasm32")]
impl GameBuffer {
    pub fn create(size: u32, usage: u32, label: &str) -> Option<Self> {
        let handle = unsafe {
            game_gpu_buffer_create(size, usage, label.as_ptr(), label.len() as u32)
        };
        if handle == 0 { None } else { Some(Self { handle }) }
    }

    pub fn write(&self, data: &[u8]) {
        unsafe { game_gpu_buffer_write(self.handle, data.as_ptr(), data.len() as u32) }
    }

    pub fn handle(&self) -> u32 {
        self.handle
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for GameBuffer {
    fn drop(&mut self) {
        unsafe { game_gpu_buffer_destroy(self.handle) }
    }
}

// Refcounted WebGPU objects (shader modules, layouts, bind groups,
// pipelines) don't have explicit .destroy() — the browser refcounts
// them via device. These wrappers hold a handle for greppability but
// don't need Drop cleanup. If a leak surfaces on the JS handle map
// later, add a generic `game_gpu_handle_release` import.

#[cfg(target_arch = "wasm32")]
pub struct GameShaderModule { handle: u32 }
#[cfg(target_arch = "wasm32")]
impl GameShaderModule {
    pub fn create(source: &str, label: &str) -> Option<Self> {
        let h = unsafe {
            game_gpu_shader_module_create(
                source.as_ptr(),
                source.len() as u32,
                label.as_ptr(),
                label.len() as u32,
            )
        };
        if h == 0 { None } else { Some(Self { handle: h }) }
    }
    pub fn handle(&self) -> u32 { self.handle }
}

#[cfg(target_arch = "wasm32")]
pub struct GameBindGroupLayout { handle: u32 }
#[cfg(target_arch = "wasm32")]
impl GameBindGroupLayout {
    /// Specialized single-vertex-uniform layout — one uniform buffer at
    /// binding 0, visibility=VERTEX. Matches render.rs's camera layout.
    pub fn create_uniform(label: &str) -> Option<Self> {
        let h = unsafe {
            game_gpu_bind_group_layout_create_uniform(label.as_ptr(), label.len() as u32)
        };
        if h == 0 { None } else { Some(Self { handle: h }) }
    }
    pub fn handle(&self) -> u32 { self.handle }
}

#[cfg(target_arch = "wasm32")]
pub struct GameBindGroup { handle: u32 }
#[cfg(target_arch = "wasm32")]
impl GameBindGroup {
    pub fn create(layout: &GameBindGroupLayout, buffer: &GameBuffer, label: &str) -> Option<Self> {
        let h = unsafe {
            game_gpu_bind_group_create(
                layout.handle,
                buffer.handle,
                label.as_ptr(),
                label.len() as u32,
            )
        };
        if h == 0 { None } else { Some(Self { handle: h }) }
    }
    pub fn handle(&self) -> u32 { self.handle }
}

#[cfg(target_arch = "wasm32")]
pub struct GamePipelineLayout { handle: u32 }
#[cfg(target_arch = "wasm32")]
impl GamePipelineLayout {
    pub fn create(bg_layout: &GameBindGroupLayout, label: &str) -> Option<Self> {
        let h = unsafe {
            game_gpu_pipeline_layout_create(bg_layout.handle, label.as_ptr(), label.len() as u32)
        };
        if h == 0 { None } else { Some(Self { handle: h }) }
    }
    pub fn handle(&self) -> u32 { self.handle }
}

/// Canvas-bound render target: WebGPU canvas context + depth texture,
/// created once via configure. The JS shim owns getCurrentTexture()
/// per frame; Rust just references it by handle.
#[cfg(target_arch = "wasm32")]
pub struct GameRenderTarget { handle: u32 }

#[cfg(target_arch = "wasm32")]
impl GameRenderTarget {
    pub fn configure(canvas_id: &str, color_format: u32, depth_format: u32) -> Option<Self> {
        let h = unsafe {
            game_gpu_render_target_configure(
                canvas_id.as_ptr(),
                canvas_id.len() as u32,
                color_format,
                depth_format,
            )
        };
        if h == 0 { None } else { Some(Self { handle: h }) }
    }
    pub fn handle(&self) -> u32 { self.handle }
}

/// Bundled encode/draw/submit — the JS shim owns the encoder + pass +
/// submit dance internally. Returns 0 on success, non-zero on error.
#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    target: &GameRenderTarget,
    pipeline: &GameRenderPipeline,
    bind_group: &GameBindGroup,
    vertex_buf: &GameBuffer,
    instance_buf: &GameBuffer,
    vertex_count: u32,
    instance_count: u32,
    clear_rgb: [f32; 3],
) -> u32 {
    unsafe {
        game_gpu_render_frame(
            target.handle,
            pipeline.handle,
            bind_group.handle,
            vertex_buf.handle,
            instance_buf.handle,
            vertex_count,
            instance_count,
            clear_rgb[0], clear_rgb[1], clear_rgb[2],
        )
    }
}

/// UI-overlay pipeline: no vertex buffer (quad verts computed from
/// @builtin(vertex_index)); one instance buffer holding UiInstance
/// with per-quad NDC center, half-size, color, alpha. Uses the same
/// bind group layout as the world pipeline for pipeline-layout reuse.
#[cfg(target_arch = "wasm32")]
pub struct GameUiPipeline { handle: u32 }

#[cfg(target_arch = "wasm32")]
impl GameUiPipeline {
    pub fn create(
        pipeline_layout: &GamePipelineLayout,
        shader: &GameShaderModule,
        instance_stride: u32,
        color_format: u32,
        label: &str,
    ) -> Option<Self> {
        let h = unsafe {
            game_gpu_render_pipeline_create_ui(
                pipeline_layout.handle,
                shader.handle,
                instance_stride,
                color_format,
                label.as_ptr(),
                label.len() as u32,
            )
        };
        if h == 0 { None } else { Some(Self { handle: h }) }
    }
    pub fn handle(&self) -> u32 { self.handle }
}

/// UI overlay render pass — LOAD the existing color attachment
/// (world already drew into it), no depth attachment, draw N
/// instanced quads via the UI pipeline. Returns 0 on success.
#[cfg(target_arch = "wasm32")]
pub fn render_ui_overlay(
    target: &GameRenderTarget,
    pipeline: &GameUiPipeline,
    bind_group: &GameBindGroup,
    instance_buf: &GameBuffer,
    instance_count: u32,
) -> u32 {
    unsafe {
        game_gpu_render_ui_overlay(
            target.handle,
            pipeline.handle,
            bind_group.handle,
            instance_buf.handle,
            instance_count,
        )
    }
}

/// Glass render pass — LOAD the world's colour AND depth attachments
/// (opaque already drew + wrote depth), alpha-blend the translucent
/// panes on top, depth-test but do not depth-write. Same vertex +
/// instance buffers as the world pass. Returns 0 on success.
#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
pub fn render_glass(
    target: &GameRenderTarget,
    pipeline: &GameRenderPipeline,
    bind_group: &GameBindGroup,
    vertex_buf: &GameBuffer,
    instance_buf: &GameBuffer,
    vertex_count: u32,
    instance_count: u32,
) -> u32 {
    unsafe {
        game_gpu_render_glass(
            target.handle,
            pipeline.handle,
            bind_group.handle,
            vertex_buf.handle,
            instance_buf.handle,
            vertex_count,
            instance_count,
        )
    }
}

#[cfg(target_arch = "wasm32")]
pub struct GameRenderPipeline { handle: u32 }
#[cfg(target_arch = "wasm32")]
impl GameRenderPipeline {
    /// Translucent glass pipeline — same cube vertex + instance layout,
    /// but alpha-blended with depth-write disabled (depth-test still on)
    /// so panes blend over the opaque world without occluding each other.
    pub fn create_glass(
        pipeline_layout: &GamePipelineLayout,
        shader: &GameShaderModule,
        vertex_stride: u32,
        instance_stride: u32,
        color_format: u32,
        depth_format: u32,
        label: &str,
    ) -> Option<Self> {
        let h = unsafe {
            game_gpu_render_pipeline_create_glass(
                pipeline_layout.handle,
                shader.handle,
                vertex_stride,
                instance_stride,
                color_format,
                depth_format,
                label.as_ptr(),
                label.len() as u32,
            )
        };
        if h == 0 { None } else { Some(Self { handle: h }) }
    }

    /// Specialized cube pipeline — matches render.rs's shape:
    /// vertex buffer at slot 0 (pos+normal, both float32x3),
    /// instance buffer at slot 1 (i_pos+i_color+i_scale, all float32x3).
    /// Triangle-list, CCW, back-cull, depth-less-write.
    pub fn create_cube(
        pipeline_layout: &GamePipelineLayout,
        shader: &GameShaderModule,
        vertex_stride: u32,
        instance_stride: u32,
        color_format: u32,
        depth_format: u32,
        label: &str,
    ) -> Option<Self> {
        let h = unsafe {
            game_gpu_render_pipeline_create_cube(
                pipeline_layout.handle,
                shader.handle,
                vertex_stride,
                instance_stride,
                color_format,
                depth_format,
                label.as_ptr(),
                label.len() as u32,
            )
        };
        if h == 0 { None } else { Some(Self { handle: h }) }
    }
    pub fn handle(&self) -> u32 { self.handle }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_from_u32_maps_documented_values() {
        assert_eq!(GpuStatus::from_u32(0), GpuStatus::Pending);
        assert_eq!(GpuStatus::from_u32(1), GpuStatus::Ready);
        assert_eq!(GpuStatus::from_u32(2), GpuStatus::Unavailable);
    }

    #[test]
    fn status_from_u32_out_of_range_is_pending() {
        assert_eq!(GpuStatus::from_u32(3), GpuStatus::Pending);
        assert_eq!(GpuStatus::from_u32(u32::MAX), GpuStatus::Pending);
    }
}
