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
use crate::scene::{
    GLASS_SHADER_WGSL, GpuVertex, LEAF_SHADER_WGSL, MESH_SHADER_WGSL, MeshInstance,
    MeshTreeInstances, SceneCamera, SceneInstance, SHADER_WGSL, as_bytes, cube_geometry,
};
use crate::tree_mesh::{self, MeshVertex};

const TARGET_SIZE: u32 = 512;
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuCamera {
    view_proj: [[f32; 4]; 4],
}

/// Render the scene to a PNG at `out_path`. Independent per invocation:
/// creates the pipeline, allocates buffers/textures, submits, reads
/// back, writes PNG, drops everything. Not optimised for repeated
/// frames — this is once-per-commit CI output, not a game loop.
pub fn render_scene(
    dev: &SeerDevice,
    queue: &Queue,
    camera: &SceneCamera,
    instances: &[SceneInstance],
    glass_instances: &[SceneInstance],
    mesh_trees: &MeshTreeInstances,
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

    // Glass instances get their own buffer (may be empty → we skip the
    // pass). Sized min 1 so the allocation never has size 0.
    let glass_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.glass.instance"),
        size: (std::mem::size_of_val(glass_instances) as u64).max(std::mem::size_of::<SceneInstance>() as u64),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !glass_instances.is_empty() {
        queue.write_buffer(glass_buf.wgpu(), 0, as_bytes(glass_instances));
    }

    // Mesh geometry + packed instance buffer. Same design as the wasm
    // path in render_web.rs: bake once, pack trunk instances then canopy
    // instances into one buffer, dispatch two indexed draw calls with
    // first_instance = trunk_count for the canopy pass.
    let (trunk_verts, trunk_indices) = tree_mesh::trunk_mesh(12, 1.0, 0.6, 1.0);
    let (canopy_verts, canopy_indices) = tree_mesh::leaf_quad_mesh();
    let trunk_vertex_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.trunk.vertex"),
        size: std::mem::size_of_val(&trunk_verts[..]) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(trunk_vertex_buf.wgpu(), 0, as_bytes(&trunk_verts));
    let trunk_index_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.trunk.index"),
        size: std::mem::size_of_val(&trunk_indices[..]) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(trunk_index_buf.wgpu(), 0, as_bytes(&trunk_indices));
    let canopy_vertex_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.canopy.vertex"),
        size: std::mem::size_of_val(&canopy_verts[..]) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(canopy_vertex_buf.wgpu(), 0, as_bytes(&canopy_verts));
    let canopy_index_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.canopy.index"),
        size: std::mem::size_of_val(&canopy_indices[..]) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(canopy_index_buf.wgpu(), 0, as_bytes(&canopy_indices));

    let total_mesh_instances =
        mesh_trees.trunks.len() + mesh_trees.canopy_elements.len();
    let mesh_instance_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.instance"),
        size: (total_mesh_instances.max(1) * std::mem::size_of::<MeshInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if total_mesh_instances > 0 {
        let mut packed: Vec<MeshInstance> = Vec::with_capacity(total_mesh_instances);
        packed.extend_from_slice(&mesh_trees.trunks);
        packed.extend_from_slice(&mesh_trees.canopy_elements);
        queue.write_buffer(mesh_instance_buf.wgpu(), 0, as_bytes(&packed));
    }

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

    // Glass pipeline — same layout, alpha-blended, depth-tested but not
    // depth-writing, so translucent panes blend over the opaque world
    // and don't occlude one another. Mirrors the wasm glass pipeline.
    let glass_shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("seer.render.glass.shader"),
        source: wgpu::ShaderSource::Wgsl(GLASS_SHADER_WGSL.into()),
    });
    let glass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("seer.render.glass.pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: glass_shader.wgpu(),
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
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: glass_shader.wgpu(),
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    // Mesh pipeline. Same pipeline_layout + bind group as the cube path
    // (shares the camera uniform), but vertex layout carries UV in the
    // extra slot (24-byte offset, float32x2) and instance attributes
    // shift to locations 3/4/5 so the vertex UV can sit at location 2.
    // Depth-write ON — trunks and canopy elements occlude each other and
    // whatever comes later (glass, ghost pass with depth Load).
    let mesh_shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("seer.render.mesh.shader"),
        source: wgpu::ShaderSource::Wgsl(MESH_SHADER_WGSL.into()),
    });
    // Leaf cards use the same vertex stage but a fragment stage that
    // carves a leaf silhouette from the quad (see LEAF_SHADER_WGSL).
    let leaf_shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("seer.render.leaf.shader"),
        source: wgpu::ShaderSource::Wgsl(LEAF_SHADER_WGSL.into()),
    });
    // One descriptor, two pipelines — trunk/branch cones and leaf cards
    // differ only in the fragment shader.
    let make_mesh_pipeline = |module: &wgpu::ShaderModule, label: &str| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<MeshVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x3,
                            2 => Float32x2
                        ],
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<MeshInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            3 => Float32x3,
                            4 => Float32x3,
                            5 => Float32x3,
                            6 => Float32x3
                        ],
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
                module,
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
        })
    };
    let mesh_pipeline = make_mesh_pipeline(mesh_shader.wgpu(), "seer.render.mesh.pipeline");
    let leaf_pipeline = make_mesh_pipeline(leaf_shader.wgpu(), "seer.render.leaf.pipeline");

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
    if total_mesh_instances > 0 {
        // Mesh pass — indexed draw, ONE pass, TWO dispatches. Trunk
        // instances live at [0..trunk_count) in the packed buffer;
        // canopy elements live at [trunk_count..total). Same pipeline,
        // same bind group; different vertex + index buffers per draw.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("seer.render.mesh.pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&mesh_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(1, mesh_instance_buf.wgpu().slice(..));
        if !mesh_trees.trunks.is_empty() {
            pass.set_vertex_buffer(0, trunk_vertex_buf.wgpu().slice(..));
            pass.set_index_buffer(trunk_index_buf.wgpu().slice(..), wgpu::IndexFormat::Uint32);
            let trunk_count = mesh_trees.trunks.len() as u32;
            pass.draw_indexed(0..trunk_indices.len() as u32, 0, 0..trunk_count);
        }
        if !mesh_trees.canopy_elements.is_empty() {
            pass.set_pipeline(&leaf_pipeline);
            pass.set_vertex_buffer(0, canopy_vertex_buf.wgpu().slice(..));
            pass.set_index_buffer(canopy_index_buf.wgpu().slice(..), wgpu::IndexFormat::Uint32);
            let trunk_count = mesh_trees.trunks.len() as u32;
            let canopy_count = mesh_trees.canopy_elements.len() as u32;
            pass.draw_indexed(
                0..canopy_indices.len() as u32,
                0,
                trunk_count..trunk_count + canopy_count,
            );
        }
    }
    if !glass_instances.is_empty() {
        // Glass pass: load the opaque colour + depth, blend the panes.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("seer.render.glass.pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&glass_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buf.wgpu().slice(..));
        pass.set_vertex_buffer(1, glass_buf.wgpu().slice(..));
        pass.draw(0..vertices.len() as u32, 0..glass_instances.len() as u32);
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
        "[render] wrote {out_path} ({} cube instances, {} vertices, {} mesh trunks, {} mesh canopy elements)",
        instances.len(),
        vertices.len(),
        mesh_trees.trunks.len(),
        mesh_trees.canopy_elements.len(),
    ));
    Ok(())
}

