// Real wgpu render path for seer's per-commit frame.png. Replaces
// the CPU-rasterized minimap with an offscreen 3D render: ground
// plane + trees + obstacles + player, drawn as instanced cubes with
// per-face normals, shaded by a single directional light plus flat
// ambient. Depth-buffered.
//
// Founding constraints of this module:
//   - Every wgpu resource (buffers, textures, shader) goes through
//     SeerDevice wrappers so obs::gpu_totals sees them and the drop
//     path calls .destroy() before wgpu forgets them.
//   - No wasm-bindgen. Native only (cfg gate lives at the module
//     boundary in lib.rs); the wasm target still emits observability
//     without touching wgpu.
//   - One draw call: cube geometry as a shared vertex buffer, every
//     scene element rendered as one instance. Ground, trees,
//     obstacles, player all use the same pipeline — differing only
//     in per-instance position, colour, scale.
//   - The frame gets written to `out_path` as a 512x512 PNG.

use anyhow::{Context, Result};
use wgpu::{Device, Queue};

use crate::gpu::SeerDevice;
use crate::obs;

const TARGET_SIZE: u32 = 512;
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Top-down-with-tilt camera parameters. Frustum spans the world so
/// [-FLOOR_HALF, +FLOOR_HALF] on XZ maps to the whole image.
pub struct SceneCamera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub half_extent: f32,
    pub near: f32,
    pub far: f32,
}

impl SceneCamera {
    pub fn default_for_floor(floor_half: f32) -> Self {
        Self {
            // Above the scene, offset backward in world Z so trees
            // and obstacles catch some side-light instead of showing
            // as pure top-down blobs.
            eye: [0.0, floor_half * 2.0, floor_half * 0.8],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            // A bit larger than the floor so trees at the edge stay
            // visible even after camera tilt shifts their screen
            // position slightly inward.
            half_extent: floor_half * 1.1,
            near: 100.0,
            far: floor_half * 5.0,
        }
    }

    /// Follow-cam anchored on the player position. Eye sits above +
    /// slightly Z-forward of the target (same tilt as default_for_floor);
    /// zoom is tighter than the full-floor default so the player
    /// occupies a meaningful chunk of the frame while still showing
    /// its immediate tree/obstacle neighbourhood. `floor_half` still
    /// bounds the near/far frustum so distant trees stay unclipped.
    pub fn follow(player: [f32; 3], floor_half: f32) -> Self {
        Self {
            eye: [
                player[0],
                player[1] + floor_half * 2.0,
                player[2] + floor_half * 0.8,
            ],
            target: player,
            up: [0.0, 1.0, 0.0],
            half_extent: floor_half * 0.6,
            near: 100.0,
            far: floor_half * 5.0,
        }
    }

    /// view_proj packed row-major → matches WGSL mat4x4<f32> upload.
    fn view_proj(&self) -> [[f32; 4]; 4] {
        let eye = bevy_math::Vec3::new(self.eye[0], self.eye[1], self.eye[2]);
        let target = bevy_math::Vec3::new(self.target[0], self.target[1], self.target[2]);
        let up = bevy_math::Vec3::new(self.up[0], self.up[1], self.up[2]);
        let view = bevy_math::Mat4::look_at_rh(eye, target, up);
        let proj = bevy_math::Mat4::orthographic_rh(
            -self.half_extent,
            self.half_extent,
            -self.half_extent,
            self.half_extent,
            self.near,
            self.far,
        );
        (proj * view).to_cols_array_2d()
    }
}

/// One draw-call instance. Positions are world coords; scale is per-
/// axis stretch of the unit cube (-0.5..0.5); color is linear RGB.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SceneInstance {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub scale: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuVertex {
    pos: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuCamera {
    view_proj: [[f32; 4]; 4],
}

fn cube_geometry() -> Vec<GpuVertex> {
    // Each face: outward normal, in-plane u axis, in-plane v axis.
    // Winding: c00→c10→c11 then c00→c11→c01 in the (u,v) frame,
    // giving CCW-from-outside for every face (default front face).
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),   // +Z
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // -Z
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),  // +X
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),  // -X
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),  // +Y
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),  // -Y
    ];
    let mut out: Vec<GpuVertex> = Vec::with_capacity(36);
    for (n, u, v) in faces {
        let center = [n[0] * 0.5, n[1] * 0.5, n[2] * 0.5];
        let mk = |su: f32, sv: f32| GpuVertex {
            pos: [
                center[0] + u[0] * su * 0.5 + v[0] * sv * 0.5,
                center[1] + u[1] * su * 0.5 + v[1] * sv * 0.5,
                center[2] + u[2] * su * 0.5 + v[2] * sv * 0.5,
            ],
            normal: n,
        };
        let c00 = mk(-1.0, -1.0);
        let c10 = mk(1.0, -1.0);
        let c11 = mk(1.0, 1.0);
        let c01 = mk(-1.0, 1.0);
        out.push(c00);
        out.push(c10);
        out.push(c11);
        out.push(c00);
        out.push(c11);
        out.push(c01);
    }
    out
}

fn as_bytes<T: Copy>(slice: &[T]) -> &[u8] {
    // Safety: T is repr(C) Copy in every caller here, so its slice
    // representation is a valid byte sequence with no padding
    // surprises across the lifetime of the borrow.
    unsafe {
        std::slice::from_raw_parts(slice.as_ptr() as *const u8, std::mem::size_of_val(slice))
    }
}

const SHADER_WGSL: &str = r#"
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct IIn {
    @location(2) i_pos: vec3<f32>,
    @location(3) i_color: vec3<f32>,
    @location(4) i_scale: vec3<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs(v: VIn, i: IIn) -> VOut {
    let world = v.pos * i.i_scale + i.i_pos;
    var o: VOut;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    // Cubes are uniformly axis-aligned; instance scale doesn't rotate
    // normals, only stretches them along the same axes, so the
    // direction stays correct without a normal-matrix transform.
    o.normal = normalize(v.normal);
    o.color = i.i_color;
    return o;
}

const LIGHT_DIR: vec3<f32> = vec3<f32>(0.3, 0.85, 0.4);
const AMBIENT: f32 = 0.25;

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    return vec4<f32>(in.color * k, 1.0);
}
"#;

/// Render the scene to a PNG at `out_path`. Independent per invocation:
/// creates the pipeline, allocates buffers/textures, submits, reads
/// back, writes PNG, drops everything. Not optimised for repeated
/// frames — this is once-per-commit CI output, not a game loop.
pub fn render_scene(
    dev: &SeerDevice,
    queue: &Queue,
    camera: &SceneCamera,
    instances: &[SceneInstance],
    out_path: &str,
) -> Result<()> {
    let device: &Device = dev.wgpu();

    let vertices = cube_geometry();
    let vertex_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.vertex"),
        size: std::mem::size_of_val(&vertices[..]) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(vertex_buf.wgpu(), 0, as_bytes(&vertices));

    let instance_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.instance"),
        size: std::mem::size_of_val(instances) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(instance_buf.wgpu(), 0, as_bytes(instances));

    let camera_data = GpuCamera {
        view_proj: camera.view_proj(),
    };
    let camera_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.camera"),
        size: std::mem::size_of::<GpuCamera>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(camera_buf.wgpu(), 0, as_bytes(std::slice::from_ref(&camera_data)));

    let color_tex = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("seer.render.color"),
        size: wgpu::Extent3d {
            width: TARGET_SIZE,
            height: TARGET_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_tex
        .wgpu()
        .create_view(&wgpu::TextureViewDescriptor::default());

    let depth_tex = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("seer.render.depth"),
        size: wgpu::Extent3d {
            width: TARGET_SIZE,
            height: TARGET_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_tex
        .wgpu()
        .create_view(&wgpu::TextureViewDescriptor::default());

    let shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("seer.render.shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("seer.render.bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("seer.render.bg"),
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buf.wgpu().as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("seer.render.pl"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("seer.render.pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader.wgpu(),
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SceneInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32x3, 4 => Float32x3],
                },
            ],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader.wgpu(),
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    // Readback path: color texture → staging buffer → CPU → PNG.
    // Row pitch must be a multiple of 256 bytes (COPY_BYTES_PER_ROW
    // _ALIGNMENT); at 512 * 4 = 2048 it already is, so no padding.
    let bytes_per_row = TARGET_SIZE * 4;
    let staging_size = (bytes_per_row * TARGET_SIZE) as u64;
    let staging = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.staging"),
        size: staging_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("seer.render.encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("seer.render.pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.040,
                        g: 0.055,
                        b: 0.080,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buf.wgpu().slice(..));
        pass.set_vertex_buffer(1, instance_buf.wgpu().slice(..));
        pass.draw(0..vertices.len() as u32, 0..instances.len() as u32);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: color_tex.wgpu(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: staging.wgpu(),
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(TARGET_SIZE),
            },
        },
        wgpu::Extent3d {
            width: TARGET_SIZE,
            height: TARGET_SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let (tx, rx) = std::sync::mpsc::channel();
    staging.wgpu().slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .context("wgpu poll failed")?;
    rx.recv().context("wgpu map_async recv")?.context("wgpu map_async result")?;

    let pixels: Vec<u8> = staging.wgpu().slice(..).get_mapped_range().to_vec();
    staging.wgpu().unmap();

    // PNG encode. seer stays on the png crate so we don't add an image
    // dep — a raw RGBA buffer at fixed size is straightforward.
    let file = std::fs::File::create(out_path)
        .with_context(|| format!("creating {out_path}"))?;
    let bw = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(bw, TARGET_SIZE, TARGET_SIZE);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("png header")?;
    writer.write_image_data(&pixels).context("png write")?;

    obs::emit(&format!(
        "[render] wrote {out_path} ({} instances, {} vertices)",
        instances.len(),
        vertices.len()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_geometry_is_36_verts_on_unit_bounds_with_unit_normals() {
        let verts = cube_geometry();
        assert_eq!(verts.len(), 36);
        for v in &verts {
            for c in v.pos {
                assert!(c.abs() <= 0.5 + 1e-6, "vertex out of unit cube: {:?}", v.pos);
            }
            let n2: f32 = v.normal.iter().map(|c| c * c).sum();
            assert!((n2 - 1.0).abs() < 1e-4, "normal not unit: {:?}", v.normal);
        }
    }

    #[test]
    fn view_proj_maps_origin_into_clip_space() {
        let cam = SceneCamera::default_for_floor(3000.0);
        let vp = cam.view_proj();
        // Manually multiply the mat4 by (0,0,0,1) — origin should be
        // inside the frustum (all clip coords in [-1, 1] for x/y and
        // [0, 1] for z with orthographic_rh).
        let col_major = |m: &[[f32; 4]; 4], r: usize| -> f32 {
            // to_cols_array_2d produces column-major; index [col][row].
            m[0][r] * 0.0 + m[1][r] * 0.0 + m[2][r] * 0.0 + m[3][r] * 1.0
        };
        let cx = col_major(&vp, 0);
        let cy = col_major(&vp, 1);
        let cz = col_major(&vp, 2);
        let cw = col_major(&vp, 3);
        // Orthographic: w is always 1, no perspective divide needed.
        assert!((cw - 1.0).abs() < 1e-4, "ortho w should be 1, got {cw}");
        assert!(cx.abs() < 1.0, "origin should be in-frustum x: {cx}");
        assert!(cy.abs() < 1.0, "origin should be in-frustum y: {cy}");
        assert!((0.0..=1.0).contains(&cz), "origin should be in-frustum z: {cz}");
    }

    #[test]
    fn follow_camera_maps_player_to_ndc_center() {
        let player = [1000.0, 20.0, -500.0];
        let cam = SceneCamera::follow(player, 3000.0);
        let vp = cam.view_proj();
        // Applying view_proj to the player world position must land
        // on the near-center of the clip volume — that's the whole
        // point of "follow": target == player.
        let col_major = |m: &[[f32; 4]; 4], r: usize| -> f32 {
            m[0][r] * player[0] + m[1][r] * player[1] + m[2][r] * player[2] + m[3][r] * 1.0
        };
        let cx = col_major(&vp, 0);
        let cy = col_major(&vp, 1);
        let cw = col_major(&vp, 3);
        assert!((cw - 1.0).abs() < 1e-4, "ortho w should be 1, got {cw}");
        assert!(cx.abs() < 1e-3, "player should be at NDC x=0, got {cx}");
        assert!(cy.abs() < 1e-3, "player should be at NDC y=0, got {cy}");
    }
}
