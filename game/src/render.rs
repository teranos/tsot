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
use crate::scene::{GpuVertex, MeshInstance, SceneCamera, SceneInstance, as_bytes, cube_geometry};
use crate::shaders::{GLASS_SHADER_WGSL, LEAF_SHADER_WGSL, MESH_SHADER_WGSL, SHADER_WGSL};
use crate::tree_emit::MeshTreeInstances;
use crate::tree_mesh::{self, MeshVertex};

const TARGET_SIZE: u32 = 512;
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuCamera {
    view_proj: [[f32; 4]; 4],
    /// `x` = elapsed seconds driving leaf-wind sway (synthetic ticks, no
    /// `bevy_time`); `yzw` spare. Matches `Camera.wind` in the mesh/leaf
    /// WGSL. Non-mesh shaders declare only `view_proj` and ignore the
    /// extra 16 bytes — a larger uniform buffer than a shader reads is
    /// valid.
    wind: [f32; 4],
}

/// Render the scene to a PNG at `out_path`. Independent per invocation:
/// creates the pipeline, allocates buffers/textures, submits, reads
/// back, writes PNG, drops everything. Not optimised for repeated
/// frames — this is once-per-commit CI output, not a game loop.
// A GPU render entry point: device, queue, camera, three distinct
// instance sets, time, and an output path are each genuinely their own
// argument — grouping them into a struct would be an artificial vehicle.
#[allow(clippy::too_many_arguments)]
pub fn render_scene(
    dev: &SeerDevice,
    queue: &Queue,
    camera: &SceneCamera,
    instances: &[SceneInstance],
    glass_instances: &[SceneInstance],
    mesh_trees: &MeshTreeInstances,
    grid: &[MeshInstance],
    surface: &crate::scene::TerrainSurface,
    time: f32,
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

    // Canopy geometry (unit leaf card, baked once). Wood is per-species:
    // each species' mesh (`tree_surface::species_wood_mesh`) gets one
    // vertex+index buffer, drawn instanced per tree. See docs/TREES.md.
    let (canopy_verts, canopy_indices) = tree_mesh::leaf_quad_mesh();
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

    // One vertex+index+instance buffer per species. Buffers live only
    // for this render call (native seer runs one snapshot at a time),
    // so no long-lived state needed here.
    struct SpeciesBufs {
        vertex_buf: crate::gpu::SeerBuffer,
        index_buf: crate::gpu::SeerBuffer,
        instance_buf: crate::gpu::SeerBuffer,
        index_count: u32,
        instance_count: u32,
    }
    let mut species_bufs: Vec<SpeciesBufs> = Vec::with_capacity(mesh_trees.wood_by_species.len());
    for (sp, instances) in &mesh_trees.wood_by_species {
        let mesh = crate::tree_surface::species_wood_mesh(sp);
        let vertex_buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("seer.render.mesh.wood.vertex"),
            size: std::mem::size_of_val(&mesh.0[..]) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(vertex_buf.wgpu(), 0, as_bytes(&mesh.0));
        let index_buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("seer.render.mesh.wood.index"),
            size: std::mem::size_of_val(&mesh.1[..]) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(index_buf.wgpu(), 0, as_bytes(&mesh.1));
        let instance_buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("seer.render.mesh.wood.instance"),
            size: std::mem::size_of_val(&instances[..]) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(instance_buf.wgpu(), 0, as_bytes(instances));
        species_bufs.push(SpeciesBufs {
            vertex_buf,
            index_buf,
            instance_buf,
            index_count: mesh.1.len() as u32,
            instance_count: instances.len() as u32,
        });
    }

    // Canopy instances get their own instance buffer, unrelated to wood.
    let canopy_len = mesh_trees.canopy_elements.len();
    let canopy_instance_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.canopy.instance"),
        size: (canopy_len.max(1) * std::mem::size_of::<MeshInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if canopy_len > 0 {
        queue.write_buffer(
            canopy_instance_buf.wgpu(),
            0,
            as_bytes(&mesh_trees.canopy_elements),
        );
    }

    // Draped dev-grid: one shared unit bar, instanced per segment, drawn
    // through the mesh pipeline (see scene::dev_grid_mesh, TERRAIN.md).
    let (grid_verts, grid_indices) = tree_mesh::unit_bar_mesh();
    let grid_vertex_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.grid.vertex"),
        size: std::mem::size_of_val(&grid_verts[..]) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(grid_vertex_buf.wgpu(), 0, as_bytes(&grid_verts));
    let grid_index_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.grid.index"),
        size: std::mem::size_of_val(&grid_indices[..]) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(grid_index_buf.wgpu(), 0, as_bytes(&grid_indices));
    let grid_len = grid.len();
    let grid_instance_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.grid.instance"),
        size: (grid_len.max(1) * std::mem::size_of::<MeshInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if grid_len > 0 {
        queue.write_buffer(grid_instance_buf.wgpu(), 0, as_bytes(grid));
    }

    // Solid terrain surface — one mesh drawn once (identity instance).
    let surf_len = surface.indices.len();
    let surface_vertex_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.surface.vertex"),
        size: (std::mem::size_of_val(&surface.verts[..]) as u64).max(4),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let surface_index_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.surface.index"),
        size: (std::mem::size_of_val(&surface.indices[..]) as u64).max(4),
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let surface_instance = [MeshInstance {
        pos: [0.0, 0.0, 0.0],
        color: [0.20, 0.26, 0.15], // one mossy ground colour; Lambert does the rest
        scale: [1.0, 1.0, 1.0],
        axis: [0.0, 1.0, 0.0, 0.0], // identity — surface verts are world-space
    }];
    let surface_instance_buf = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render.mesh.surface.instance"),
        size: std::mem::size_of_val(&surface_instance) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if surf_len > 0 {
        queue.write_buffer(surface_vertex_buf.wgpu(), 0, as_bytes(&surface.verts));
        queue.write_buffer(surface_index_buf.wgpu(), 0, as_bytes(&surface.indices));
        queue.write_buffer(surface_instance_buf.wgpu(), 0, as_bytes(&surface_instance));
    }

    let tp = crate::tune::get();
    let camera_data = GpuCamera {
        view_proj: camera.view_proj(),
        wind: [time, tp.wind_amp, tp.wind_speed, tp.wind_leaf_mult],
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
                            6 => Float32x4
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
    let has_mesh_work =
        !species_bufs.is_empty() || canopy_len > 0 || grid_len > 0 || surf_len > 0;
    if has_mesh_work {
        // Mesh pass — one dispatch per species drawing that species'
        // wood mesh instanced across its trees, then one canopy
        // dispatch. Same pipeline layout throughout.
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
        // Solid ground first — the grid and props draw over it.
        if surf_len > 0 {
            pass.set_vertex_buffer(0, surface_vertex_buf.wgpu().slice(..));
            pass.set_vertex_buffer(1, surface_instance_buf.wgpu().slice(..));
            pass.set_index_buffer(surface_index_buf.wgpu().slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..surf_len as u32, 0, 0..1);
        }
        for sb in &species_bufs {
            pass.set_vertex_buffer(0, sb.vertex_buf.wgpu().slice(..));
            pass.set_vertex_buffer(1, sb.instance_buf.wgpu().slice(..));
            pass.set_index_buffer(sb.index_buf.wgpu().slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..sb.index_count, 0, 0..sb.instance_count);
        }
        // Draped dev-grid — shared unit bar, one instance per segment.
        if grid_len > 0 {
            pass.set_vertex_buffer(0, grid_vertex_buf.wgpu().slice(..));
            pass.set_vertex_buffer(1, grid_instance_buf.wgpu().slice(..));
            pass.set_index_buffer(grid_index_buf.wgpu().slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..grid_indices.len() as u32, 0, 0..grid_len as u32);
        }
        if canopy_len > 0 {
            pass.set_pipeline(&leaf_pipeline);
            pass.set_vertex_buffer(0, canopy_vertex_buf.wgpu().slice(..));
            pass.set_vertex_buffer(1, canopy_instance_buf.wgpu().slice(..));
            pass.set_index_buffer(canopy_index_buf.wgpu().slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..canopy_indices.len() as u32, 0, 0..canopy_len as u32);
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

    let wood_tree_count: usize = mesh_trees
        .wood_by_species
        .iter()
        .map(|(_, v)| v.len())
        .sum();
    obs::emit(&format!(
        "[render] wrote {out_path} ({} cube instances, {} vertices, {} wood species / {} wood trees, {} canopy elements)",
        instances.len(),
        vertices.len(),
        mesh_trees.wood_by_species.len(),
        wood_tree_count,
        mesh_trees.canopy_elements.len(),
    ));
    Ok(())
}


#[cfg(test)]
mod tests {
    use crate::scene::INSTANCE_ATTRS;

    #[test]
    fn native_mesh_instance_attrs_derive_from_the_source() {
        // The native attribute array (the `vertex_attr_array!` in the mesh
        // pipeline) must match the single source of truth. wgpu computes
        // the offsets from the format sequence; assert they equal what
        // INSTANCE_ATTRS declares, so a change to one fails the build gate.
        let native = wgpu::vertex_attr_array![
            3 => Float32x3, 4 => Float32x3, 5 => Float32x3, 6 => Float32x4
        ];
        assert_eq!(native.len(), INSTANCE_ATTRS.len());
        let js_fmt = |f: wgpu::VertexFormat| match f {
            wgpu::VertexFormat::Float32x3 => "float32x3",
            wgpu::VertexFormat::Float32x4 => "float32x4",
            other => panic!("unmapped instance format {other:?}"),
        };
        for (n, a) in native.iter().zip(INSTANCE_ATTRS) {
            assert_eq!(n.shader_location, a.location);
            assert_eq!(n.offset, a.offset);
            assert_eq!(js_fmt(n.format), a.format);
        }
    }
}
