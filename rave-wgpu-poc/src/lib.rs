//! rave-wgpu-poc — a minimal slice of the rave world rendered with
//! wgpu 29 directly, no Bevy, no winit.
//!
//! What it proves (or disproves) in one wasm binary:
//!   1. A plain LDR forward scene at real mobile resolution renders on
//!      the phone — the case Bevy-rave fails (goes black / OOMs).
//!   2. ONE binary carries both WebGPU and WebGL2 and picks at runtime
//!      (`?backend=webgpu|webgl2` forces one), removing Bevy's
//!      compile-time two-bundle split.
//!
//! The scene (forest floor + Wang-hash trees + follow camera) and its
//! constants are ported 1:1 from the Bevy source so the GPU load is a
//! fair match. See `scene.rs`.

pub mod mesh;
pub mod scene;

#[cfg(target_arch = "wasm32")]
mod app {
    use std::cell::RefCell;
    use std::rc::Rc;

    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use wgpu::util::DeviceExt;

    use crate::mesh::{CameraUniform, Instance, MeshData, Vertex};
    use crate::scene;

    fn window() -> web_sys::Window {
        web_sys::window().expect("no global window")
    }

    fn set_status(text: &str) {
        if let Some(doc) = window().document() {
            if let Some(el) = doc.get_element_by_id("status") {
                el.set_text_content(Some(text));
            }
        }
    }

    fn request_animation_frame(f: &Closure<dyn FnMut()>) {
        window()
            .request_animation_frame(f.as_ref().unchecked_ref())
            .expect("requestAnimationFrame failed");
    }

    fn now_secs(start_ms: f64) -> f32 {
        let perf = window().performance().expect("no performance");
        ((perf.now() - start_ms) / 1000.0) as f32
    }

    /// One uploaded mesh: vertex + index buffer + index count.
    struct GpuMesh {
        vbuf: wgpu::Buffer,
        ibuf: wgpu::Buffer,
        count: u32,
    }

    impl GpuMesh {
        fn upload(device: &wgpu::Device, m: &MeshData) -> Self {
            let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vbuf"),
                contents: bytemuck::cast_slice(&m.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ibuf"),
                contents: bytemuck::cast_slice(&m.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
            Self { vbuf, ibuf, count: m.indices.len() as u32 }
        }
    }

    fn instance_buffer(device: &wgpu::Device, data: &[Instance]) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instances"),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    fn depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    struct State {
        surface: wgpu::Surface<'static>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: wgpu::SurfaceConfiguration,
        pipeline: wgpu::RenderPipeline,
        camera_buf: wgpu::Buffer,
        camera_bg: wgpu::BindGroup,
        depth: wgpu::TextureView,
        floor_mesh: GpuMesh,
        trunk_mesh: GpuMesh,
        sphere_mesh: GpuMesh,
        floor_inst: (wgpu::Buffer, u32),
        trunk_inst: (wgpu::Buffer, u32),
        sphere_inst: (wgpu::Buffer, u32),
        backend_label: String,
        adapter_label: String,
        tree_count: usize,
        start_ms: f64,
        frames: u32,
        last_report_s: f32,
    }

    pub async fn run() {
        let doc = window().document().expect("no document");
        let canvas = doc
            .get_element_by_id("bevy") // reuse rave's canvas id for drop-in parity
            .or_else(|| doc.get_element_by_id("canvas"))
            .expect("no #bevy / #canvas element")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("target is not a canvas");

        let width = canvas.width().max(1);
        let height = canvas.height().max(1);

        // ?backend=webgpu|webgl2 forces one backend; default lets wgpu
        // try WebGPU and fall back to WebGL2 — from ONE binary.
        let search = window().location().search().unwrap_or_default();
        let backends = if search.contains("backend=webgl2") {
            wgpu::Backends::GL
        } else if search.contains("backend=webgpu") {
            wgpu::Backends::BROWSER_WEBGPU
        } else {
            wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL
        };

        set_status("[poc] creating wgpu instance…");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });

        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .expect("create_surface from canvas failed");

        set_status("[poc] requesting adapter…");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .expect("no GPU adapter — requestAdapter returned null/err");

        let info = adapter.get_info();
        let backend_label = format!("{:?}", info.backend);
        let adapter_label = format!("{} ({:?})", info.name, info.device_type);

        set_status(&format!("[poc] adapter: {adapter_label} · {backend_label} — requesting device…"));

        // downlevel_webgl2 limits so the SAME device config is valid on
        // both backends — no HDR, no MSAA, no storage textures.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("rave-poc-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("request_device failed");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // --- pipeline ---
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forward"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pl"),
            bind_group_layouts: &[Some(&bg_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("forward-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Vertex::layout(), Instance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- geometry + instances ---
        let floor_mesh = GpuMesh::upload(&device, &MeshData::quad());
        let trunk_mesh = GpuMesh::upload(&device, &MeshData::cylinder(8));
        let sphere_mesh = GpuMesh::upload(&device, &MeshData::sphere(8, 10));

        let scene = scene::build();
        let floor_inst = (instance_buffer(&device, &scene.floor), scene.floor.len() as u32);
        let trunk_inst = (instance_buffer(&device, &scene.trunks), scene.trunks.len() as u32);
        let sphere_inst = (instance_buffer(&device, &scene.spheres), scene.spheres.len() as u32);
        let tree_count = scene.tree_count;

        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera-bg"),
            layout: &bg_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        let depth = depth_view(&device, width, height);

        let mut state = State {
            surface,
            device,
            queue,
            config,
            pipeline,
            camera_buf,
            camera_bg,
            depth,
            floor_mesh,
            trunk_mesh,
            sphere_mesh,
            floor_inst,
            trunk_inst,
            sphere_inst,
            backend_label,
            adapter_label,
            tree_count,
            start_ms: window().performance().expect("perf").now(),
            frames: 0,
            last_report_s: 0.0,
        };

        // --- RAF loop ---
        let f = Rc::new(RefCell::new(None::<Closure<dyn FnMut()>>));
        let g = f.clone();
        *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
            state.frame();
            request_animation_frame(f.borrow().as_ref().unwrap());
        }) as Box<dyn FnMut()>));
        request_animation_frame(g.borrow().as_ref().unwrap());
    }

    impl State {
        fn frame(&mut self) {
            let t = now_secs(self.start_ms);
            self.frames += 1;

            let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
            let vp = scene::view_proj(t, aspect);
            let cam = CameraUniform {
                view_proj: vp.to_cols_array_2d(),
                // Late-afternoon light raking across the forest.
                light_dir: [0.4, 0.8, 0.3, 0.0],
                ambient: [0.25, 0.0, 0.0, 0.0],
            };
            self.queue
                .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&cam));

            let frame = match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(f)
                | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                _ => {
                    set_status("[poc] surface not ready (outdated/lost/timeout) — reconfiguring");
                    self.surface.configure(&self.device, &self.config);
                    return;
                }
            };
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut enc = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("enc") });
            {
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("main"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            // rave's night-sky clear (srgb 0.01,0.05,0.12).
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.01,
                                g: 0.05,
                                b: 0.12,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.depth,
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
                rp.set_pipeline(&self.pipeline);
                rp.set_bind_group(0, &self.camera_bg, &[]);

                for (m, inst) in [
                    (&self.floor_mesh, &self.floor_inst),
                    (&self.trunk_mesh, &self.trunk_inst),
                    (&self.sphere_mesh, &self.sphere_inst),
                ] {
                    rp.set_vertex_buffer(0, m.vbuf.slice(..));
                    rp.set_vertex_buffer(1, inst.0.slice(..));
                    rp.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    rp.draw_indexed(0..m.count, 0, 0..inst.1);
                }
            }
            self.queue.submit(Some(enc.finish()));
            frame.present();

            // Report once a second — a still screenshot then carries
            // backend / adapter / resolution / FPS / draw counts.
            if t - self.last_report_s >= 1.0 {
                let fps = self.frames as f32 / t.max(0.001);
                set_status(&format!(
                    "[poc] LIVE · backend={} · {} · {}x{} · trees={} · draws=3 · instances={} · ~{:.0} fps",
                    self.backend_label,
                    self.adapter_label,
                    self.config.width,
                    self.config.height,
                    self.tree_count,
                    self.floor_inst.1 + self.trunk_inst.1 + self.sphere_inst.1,
                    fps,
                ));
                self.last_report_s = t;
            }
        }
    }

    #[wasm_bindgen(start)]
    pub fn start() {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Info);
        set_status("[poc] booting…");
        wasm_bindgen_futures::spawn_local(run());
    }
}
