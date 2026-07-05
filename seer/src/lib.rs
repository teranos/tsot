// seer — cross-target entry point. Same `run()` on native and wasm.
//
// Global allocator is the instrumented one from `obs.rs` — every heap
// allocation increments counters, and every allocation >= 64 KB captures
// its call site into a bounded ring. This is architectural, not
// opt-in: the moment the runtime touches this module, observability is
// on.
//
// Commit #1 scope: no Bevy yet, no wgpu, no game logic. Prove the
// architectural spine: two targets, one obs bus, one emit sink, one
// host that observes the wasm boundary. Bevy plugs into this spine in
// the next commit; wgpu wrapper the one after; game logic after that.

pub mod build_info;
pub mod error;
pub mod health;
pub mod obs;
pub mod physics;
pub mod room;
pub mod trees;

#[cfg(not(target_arch = "wasm32"))]
pub mod gpu;

#[global_allocator]
static ALLOC: obs::InstrumentedAlloc = obs::InstrumentedAlloc;

// Wasm export. seer-host looks up `run` on the instance and calls it.
// Plain no-mangle pub extern "C" — no wasm-bindgen glue, no thousand
// imports. Every wasm→host crossing is deliberately named.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    _run();
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run() {
    _run();
}

// Phase 2: real Bevy ECS. The frame loop is now a Bevy schedule, the
// per-frame allocations are systems, retention lives in a Resource.
// Same obs bus, same instrumented allocator — but now flowing through
// bevy_ecs so we can watch what the ECS actually costs.
//
// Deliberately no bevy_time / bevy_log / bevy_render / bevy_winit yet.
// Those all pull in platform integration that we want to see cost of
// before adopting. Phase 3 wires wgpu; time/render/window come later.

use bevy_app::{App, Startup, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_math::Vec3;

use physics::{AabbCollider, PlayerMarker, Position, Velocity};

#[derive(Resource, Default)]
struct FrameCount(u32);

#[derive(Resource, Default)]
struct Retained(Vec<Vec<u8>>);

#[derive(Resource, Default)]
struct GpuHandles {
    // Per-frame churn resources — recreated each frame, destroyed at end.
    cluster: Vec<u64>,
    uniform: Vec<u64>,
}

const DEFAULT_FRAMES: u32 = 300;
const REPORT_EVERY: u32 = 30;

fn frame_budget() -> u32 {
    // SEER_FRAMES env var lets CI keep the runtime bounded and the
    // build log short. Local dev sticks with the default.
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var("SEER_FRAMES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_FRAMES)
    }
    #[cfg(target_arch = "wasm32")]
    {
        DEFAULT_FRAMES
    }
}

fn setup(mut commands: Commands) {
    obs::emit(&format!(
        "[seer.build_info] commit={} built_at={}",
        build_info::COMMIT,
        build_info::BUILT_AT
    ));
    obs::emit("[bevy.setup] Startup schedule running");
    commands.insert_resource(FrameCount::default());
    commands.insert_resource(Retained::default());
    commands.insert_resource(GpuHandles::default());
    let sid = obs::shader_created(4096, "seer.pbr");
    obs::emit(&format!(
        "[seer.setup] created shader id={sid} for demo — stays live forever"
    ));

    // Demonstrate the sacred-error bus is live: emit one Info-severity
    // record at startup so the report has evidence the drain path
    // works even in the no-real-errors baseline case.
    error::emit_region(
        error::Severity::Info,
        "seer.setup",
        "seer booted",
        format!(
            "commit={} — sacred-error bus armed",
            build_info::COMMIT
        ),
    );


    // Ported from rave: spawn a player + 5 static obstacles that the
    // resolve_collisions system iterates every frame. Real ECS query
    // pattern with With<PlayerMarker> / Without<PlayerMarker> filters.
    // Player spawns at rave's canonical SPAWN_POS.
    commands.spawn((
        PlayerMarker,
        Position(room::SPAWN_POS),
        Velocity(Vec3::new(1.5, 0.0, 0.7)),
    ));
    for (i, offset) in [
        Vec3::new(80.0, 0.0, 0.0),
        Vec3::new(-80.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 80.0),
        Vec3::new(0.0, 0.0, -80.0),
        Vec3::new(40.0, 0.0, 40.0),
    ]
    .into_iter()
    .enumerate()
    {
        commands.spawn((
            Position(offset),
            AabbCollider::cuboid(Vec3::new(30.0, 40.0, 30.0)),
        ));
        obs::emit(&format!(
            "[seer.setup] spawned obstacle {i} at {offset:?}"
        ));
    }
}

fn tick(
    mut count: ResMut<FrameCount>,
    retained: ResMut<Retained>,
    mut gpu: ResMut<GpuHandles>,
) {
    count.0 += 1;
    let frame = count.0;

    // Per-frame heap churn — dropped at end of system.
    let _cluster_cpu = vec![0u8; 200 * 1024];
    let _uniform_cpu = vec![0u8; 64 * 1024];

    // Base workload: both cluster storage AND uniform buffer churn
    // per frame. Destroy previous, create current. Steady state = flat.
    for id in gpu.cluster.drain(..) {
        obs::resource_destroyed(id);
    }
    for id in gpu.uniform.drain(..) {
        obs::resource_destroyed(id);
    }
    let cluster_id = obs::buffer_created(
        200 * 1024,
        0x80, /* STORAGE */
        "GpuClusterableObjectIndexListsStorage",
    );
    let uniform_id = obs::buffer_created(64 * 1024, 0x40 /* UNIFORM */, "GpuGlobalsBuffer");
    gpu.cluster.push(cluster_id);
    gpu.uniform.push(uniform_id);

    // ---- Leak-by-construction workload — commented out. ----
    // Uncomment to reproduce the growing-memory chart the report
    // showed at commit 400cc37. Retains CPU buffers on 5th/10th
    // frames + a scene texture every 10. The exact rave-style
    // signature; kept here as a controllable regression case.
    //
    // if frame.is_multiple_of(5) {
    //     retained.0.push(vec![0u8; 512 * 1024]);
    // }
    // if frame.is_multiple_of(10) {
    //     retained.0.push(vec![0u8; 1024 * 1024]);
    //     let tid = obs::texture_created(1024 * 1024, 0x04 /* SAMPLED */, "scene.diffuse");
    //     gpu.retained.push(tid);
    // }

    // Metric emission every frame — cheap host call (4 numbers), gives
    // the HTML report a dense time series for the chart. Detailed
    // text dumps stay at REPORT_EVERY intervals to keep the log
    // readable.
    let heap = obs::ALLOC_BYTES.load(std::sync::atomic::Ordering::Relaxed) as u64;
    let (gpu_live, gpu_bytes) = obs::gpu_totals();
    obs::emit_metric(frame, heap, gpu_live, gpu_bytes);

    if frame.is_multiple_of(REPORT_EVERY) {
        obs::emit(&format!(
            "[bevy.tick] frame={frame} retained_cpu_bufs={} live_cluster={} live_uniform={}",
            retained.0.len(),
            gpu.cluster.len(),
            gpu.uniform.len(),
        ));
        obs::dump_report();
        obs::dump_gpu_inventory();

        // Drain sacred-errors captured this window. Axiom: never
        // swallow. Info records use `[seer.note` (log-only); Warn /
        // Error / Panic use `[seer.error` so the host bucks them into
        // the report's Errors section. Same bus, different urgency.
        for e in error::drain() {
            let prefix = match e.severity {
                error::Severity::Info => "[seer.note",
                _ => "[seer.error",
            };
            obs::emit(&format!(
                "{prefix} id={} sev={:?} region={:?}] {} - {}",
                e.id, e.severity, e.context.region, e.title, e.why
            ));
        }
    }
}

fn report_player_pos(
    frame: Res<FrameCount>,
    q: Query<&Position, With<PlayerMarker>>,
) {
    if !frame.0.is_multiple_of(REPORT_EVERY) {
        return;
    }
    if let Ok(p) = q.single() {
        obs::emit(&format!(
            "[physics.player] frame={} pos=({:.2}, {:.2}, {:.2})",
            frame.0, p.0.x, p.0.y, p.0.z
        ));
    }
}

fn _run() {
    obs::emit("[seer.boot] entering run()");
    let mut app = App::new();
    app.add_systems(Startup, (setup, trees::setup_trees.after(setup)))
        .add_systems(
            Update,
            (
                physics::advance_player,
                physics::resolve_collisions.after(physics::advance_player),
                room::world_bounds_clamp.after(physics::resolve_collisions),
                tick.after(room::world_bounds_clamp),
                report_player_pos.after(tick),
            ),
        );
    let frames = frame_budget();
    obs::emit(&format!(
        "[seer.boot] Bevy App built, entering update loop for {frames} frames"
    ));
    for _ in 0..frames {
        app.update();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        native_wgpu_demo();

        // After the tick loop, snapshot the world's entity state and
        // rasterize it as a minimap PNG. Replaces the earlier static
        // WGSL triangle — the frame in each commit card now shows the
        // scene as of end-of-run, and drift across commits becomes
        // visually obvious in the gallery.
        if let Ok(out_path) = std::env::var("SEER_FRAME_PATH") {
            let world = app.world_mut();
            let mut tree_q = world.query_filtered::<&physics::Position, bevy_ecs::prelude::With<trees::TreeTrunk>>();
            let trees_vec: Vec<bevy_math::Vec3> =
                tree_q.iter(world).map(|p| p.0).collect();
            let mut obs_q = world.query_filtered::<&physics::Position, (
                bevy_ecs::prelude::With<physics::AabbCollider>,
                bevy_ecs::prelude::Without<physics::PlayerMarker>,
                bevy_ecs::prelude::Without<trees::TreeTrunk>,
            )>();
            let obstacles_vec: Vec<bevy_math::Vec3> =
                obs_q.iter(world).map(|p| p.0).collect();
            let mut player_q = world.query_filtered::<&physics::Position, bevy_ecs::prelude::With<physics::PlayerMarker>>();
            let player_pos: bevy_math::Vec3 = player_q
                .iter(world)
                .next()
                .map(|p| p.0)
                .unwrap_or(bevy_math::Vec3::ZERO);
            match render_scene_minimap(&trees_vec, &obstacles_vec, player_pos, &out_path) {
                Ok(_) => obs::emit(&format!(
                    "[gpu.native] rendered minimap ({} trees, {} obstacles, player at {:.0},{:.0}) → {out_path}",
                    trees_vec.len(),
                    obstacles_vec.len(),
                    player_pos.x,
                    player_pos.z,
                )),
                Err(e) => obs::emit(&format!("[gpu.native] minimap render failed: {e}")),
            }
        }
    }

    obs::emit("[seer.done] final report:");
    obs::dump_report();
    obs::dump_gpu_inventory();
}

#[cfg(not(target_arch = "wasm32"))]
fn native_wgpu_demo() {
    obs::emit("[gpu.native] initializing wgpu instance");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });

    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
    })) {
        Ok(a) => a,
        Err(e) => {
            obs::emit(&format!(
                "[gpu.native] no adapter available: {e:?} — skipping real wgpu demo"
            ));
            return;
        }
    };
    let info = adapter.get_info();
    obs::emit(&format!(
        "[gpu.native] adapter name={:?} backend={:?} device_type={:?}",
        info.name, info.backend, info.device_type,
    ));

    let (device, _queue): (wgpu::Device, wgpu::Queue) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("seer-native-device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        memory_hints: wgpu::MemoryHints::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        trace: wgpu::Trace::Off,
    })) {
        Ok(d) => d,
        Err(e) => {
            obs::emit(&format!(
                "[gpu.native] request_device failed: {e:?} — skipping"
            ));
            return;
        }
    };

    let dev = gpu::SeerDevice::new(device);
    obs::emit("[gpu.native] SeerDevice ready — allocating real wgpu buffers");

    let mut churned: Vec<gpu::SeerBuffer> = Vec::new();
    for i in 0..5 {
        let size = 128 * 1024 * (i + 1);
        let label = format!("seer-native-demo-{i}");
        let buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&label),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        churned.push(buf);
    }
    obs::emit(&format!(
        "[gpu.native] created {} real wgpu buffers — inventory:",
        churned.len()
    ));
    obs::dump_gpu_inventory();

    drop(churned);
    obs::emit(
        "[gpu.native] dropped all real buffers — SeerBuffer::drop should have emitted destroyed events:",
    );
    obs::dump_gpu_inventory();

    // Frame rendering handled by caller via render_scene_minimap;
    // this function proves wgpu wrapper + SeerDevice work but doesn't
    // own the frame output any more.
    let _ = dev;
    let _ = _queue;
}

#[cfg(not(target_arch = "wasm32"))]
pub fn render_scene_minimap(
    trees: &[bevy_math::Vec3],
    obstacles: &[bevy_math::Vec3],
    player: bevy_math::Vec3,
    out_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let w = 512u32;
    let h = 512u32;
    let mut pixels: Vec<u8> = Vec::with_capacity((w * h * 4) as usize);
    // Dark background matching the report page (--bg #0a0e14).
    let (bg_r, bg_g, bg_b) = (0x0a, 0x0e, 0x14);
    for _ in 0..(w * h) {
        pixels.extend_from_slice(&[bg_r, bg_g, bg_b, 0xff]);
    }

    // World XZ → image pixel. FLOOR_HALF is 3000 (rave's world). Fit
    // [-3000, +3000] into [0, 512]. Scale = 512/6000 ≈ 0.0853.
    let floor_half = crate::room::FLOOR_HALF;
    let world_to_px = |world_xz: f32| -> i32 {
        ((world_xz / floor_half + 1.0) * 0.5 * w as f32) as i32
    };
    let plot = |pixels: &mut [u8], cx: i32, cy: i32, radius: i32, rgb: (u8, u8, u8)| {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let x = cx + dx;
                let y = cy + dy;
                if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                    continue;
                }
                let idx = (y as usize * w as usize + x as usize) * 4;
                pixels[idx] = rgb.0;
                pixels[idx + 1] = rgb.1;
                pixels[idx + 2] = rgb.2;
                pixels[idx + 3] = 0xff;
            }
        }
    };

    // World-axis lines (subtle) — vertical + horizontal through origin.
    let (line_r, line_g, line_b) = (0x33, 0x41, 0x55);
    let cx = w as i32 / 2;
    let cy = h as i32 / 2;
    for i in 0..w as i32 {
        let idx = (cy as usize * w as usize + i as usize) * 4;
        pixels[idx] = line_r;
        pixels[idx + 1] = line_g;
        pixels[idx + 2] = line_b;
    }
    for j in 0..h as i32 {
        let idx = (j as usize * w as usize + cx as usize) * 4;
        pixels[idx] = line_r;
        pixels[idx + 1] = line_g;
        pixels[idx + 2] = line_b;
    }

    // Trees as small green dots (matching --down accent #22c55e).
    let tree_rgb = (0x22, 0xc5, 0x5e);
    for t in trees {
        let px = world_to_px(t.x);
        let py = world_to_px(t.z);
        plot(&mut pixels, px, py, 1, tree_rgb);
    }
    // Obstacles as amber dots (--accent3 #eab308).
    let obs_rgb = (0xea, 0xb3, 0x08);
    for o in obstacles {
        let px = world_to_px(o.x);
        let py = world_to_px(o.z);
        plot(&mut pixels, px, py, 2, obs_rgb);
    }
    // Player as cyan cross (--accent #22d3ee), larger.
    let player_rgb = (0x22, 0xd3, 0xee);
    let ppx = world_to_px(player.x);
    let ppy = world_to_px(player.z);
    plot(&mut pixels, ppx, ppy, 4, player_rgb);

    let file = std::fs::File::create(out_path)?;
    let bw = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(bw, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&pixels)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn triangle_wgsl(phase: f32) -> String {
    format!(
        r#"
struct VSOut {{ @builtin(position) pos: vec4<f32>, @location(0) col: vec3<f32> }};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VSOut {{
    var o: VSOut;
    let a = f32(i) * 2.09439 + 1.5708 + {phase};
    o.pos = vec4<f32>(sin(a) * 0.75, cos(a) * 0.75, 0.0, 1.0);
    o.col = vec3<f32>(
        select(0.15, 0.85, i == 0u),
        select(0.15, 0.85, i == 1u),
        select(0.55, 0.95, i == 2u),
    );
    return o;
}}

@fragment
fn fs_main(in: VSOut) -> @location(0) vec4<f32> {{
    return vec4<f32>(in.col, 1.0);
}}
"#
    )
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn render_triangle_png(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    out_path: &str,
    phase: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let size = 512u32;
    let format = wgpu::TextureFormat::Rgba8Unorm;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("seer.render-target"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("seer.triangle-shader"),
        source: wgpu::ShaderSource::Wgsl(triangle_wgsl(phase).into()),
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("seer.triangle-pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let bytes_per_row = size * 4;
    let staging_size = (bytes_per_row * size) as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("seer.render-readback"),
        size: staging_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("seer.render-encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("seer.render-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.040,
                        g: 0.055,
                        b: 0.075,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let (tx, rx) = std::sync::mpsc::channel();
    staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    let _ = rx.recv();

    let pixels: Vec<u8> = staging.slice(..).get_mapped_range().to_vec();
    staging.unmap();

    let file = std::fs::File::create(out_path)?;
    let bw = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(bw, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&pixels)?;
    Ok(())
}
