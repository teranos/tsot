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

#[cfg(not(target_arch = "wasm32"))]
pub mod render;

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
        if let Ok(out_path) = std::env::var("SEER_FRAME_PATH") {
            match native_render_frame(&mut app, &out_path) {
                Ok(_) => obs::emit(&format!("[gpu.native] rendered {out_path}")),
                Err(e) => obs::emit(&format!("[gpu.native] render failed: {e}")),
            }
        }
    }

    obs::emit("[seer.done] final report:");
    obs::dump_report();
    obs::dump_gpu_inventory();
}

#[cfg(not(target_arch = "wasm32"))]
fn native_render_frame(
    app: &mut App,
    out_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    obs::emit("[gpu.native] initializing wgpu");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let info = adapter.get_info();
    obs::emit(&format!(
        "[gpu.native] adapter name={:?} backend={:?} device_type={:?}",
        info.name, info.backend, info.device_type,
    ));
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("seer-native-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            trace: wgpu::Trace::Off,
        }))?;
    let dev = gpu::SeerDevice::new(device);

    // Snapshot the ECS world at end-of-run. Each of these queries is
    // filtered to the marker component the spawner assigned, so we
    // never mis-classify an obstacle as a tree or vice versa.
    let world = app.world_mut();
    let mut tree_q =
        world.query_filtered::<&physics::Position, bevy_ecs::prelude::With<trees::TreeTrunk>>();
    let trees_vec: Vec<bevy_math::Vec3> = tree_q.iter(world).map(|p| p.0).collect();
    let mut obs_q = world.query_filtered::<&physics::Position, (
        bevy_ecs::prelude::With<physics::AabbCollider>,
        bevy_ecs::prelude::Without<physics::PlayerMarker>,
        bevy_ecs::prelude::Without<trees::TreeTrunk>,
    )>();
    let obstacles_vec: Vec<bevy_math::Vec3> = obs_q.iter(world).map(|p| p.0).collect();
    let mut player_q = world
        .query_filtered::<&physics::Position, bevy_ecs::prelude::With<physics::PlayerMarker>>();
    let player_pos: bevy_math::Vec3 = player_q
        .iter(world)
        .next()
        .map(|p| p.0)
        .unwrap_or(bevy_math::Vec3::ZERO);

    // Every scene element = one instance of the shared unit cube.
    // Ground first (drawn under everything by virtue of Y position),
    // then trees, obstacles, and finally the player.
    let floor_half = room::FLOOR_HALF;
    let mut instances: Vec<render::SceneInstance> = Vec::with_capacity(
        1 + trees_vec.len() + obstacles_vec.len() + 1,
    );
    instances.push(render::SceneInstance {
        pos: [0.0, -50.0, 0.0],
        color: [0.09, 0.11, 0.15],
        scale: [floor_half * 2.0, 100.0, floor_half * 2.0],
    });
    for t in &trees_vec {
        instances.push(render::SceneInstance {
            pos: [t.x, 60.0, t.z],
            color: [0.13, 0.77, 0.37],
            scale: [40.0, 130.0, 40.0],
        });
    }
    for o in &obstacles_vec {
        instances.push(render::SceneInstance {
            pos: [o.x, 40.0, o.z],
            color: [0.92, 0.70, 0.03],
            scale: [60.0, 80.0, 60.0],
        });
    }
    instances.push(render::SceneInstance {
        pos: [player_pos.x, 60.0, player_pos.z],
        color: [0.13, 0.83, 0.93],
        scale: [70.0, 140.0, 70.0],
    });

    let camera = render::SceneCamera::default_for_floor(floor_half);
    render::render_scene(&dev, &queue, &camera, &instances, out_path)?;
    obs::emit(&format!(
        "[gpu.native] scene rendered: {} trees + {} obstacles + player @ ({:.0},{:.0})",
        trees_vec.len(),
        obstacles_vec.len(),
        player_pos.x,
        player_pos.z,
    ));
    Ok(())
}
