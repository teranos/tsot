// Game entry point. Same schedule on native and wasm; the module
// exports init/frame/finalize as separate wasm functions so a browser
// bootstrap can yield to the event loop between frames. `run()`
// bundles them for the wasmtime/native paths.

use std::cell::RefCell;

pub mod build_info;
pub mod campfire;
pub mod error;
pub mod health;
pub mod net;
pub mod obs;
pub mod physics;
pub mod room;
pub mod trees;

pub mod gpu_web;

#[cfg(not(target_arch = "wasm32"))]
pub mod gpu;

#[cfg(not(target_arch = "wasm32"))]
pub mod render;

#[global_allocator]
static ALLOC: obs::InstrumentedAlloc = obs::InstrumentedAlloc;

use bevy_app::{App, Startup, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_math::Vec3;

use physics::{AabbCollider, PlayerMarker, Position, Velocity};

// Held across init/frame/finalize calls. Single-threaded: wasm32 has
// no threads; native drives from main only.
thread_local! {
    static APP_STATE: RefCell<Option<App>> = const { RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static NATIVE_STATE: RefCell<Option<NativeRunState>> = const { RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeRunState {
    budget: u32,
    counter: u32,
    checkpoints: Vec<u32>,
    snapshots: Vec<SceneSnapshot>,
    multi_dir: Option<String>,
}

#[derive(Resource, Default)]
struct FrameCount(u32);

#[derive(Resource, Default)]
struct Retained(Vec<Vec<u8>>);

#[derive(Resource, Default)]
struct GpuHandles {
    cluster: Vec<u64>,
    uniform: Vec<u64>,
}

const DEFAULT_FRAMES: u32 = 300;
const REPORT_EVERY: u32 = 30;

fn frame_budget() -> u32 {
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

    error::emit_region(
        error::Severity::Info,
        "seer.setup",
        "seer booted",
        format!("commit={} — sacred-error bus armed", build_info::COMMIT),
    );

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

    let _cluster_cpu = vec![0u8; 200 * 1024];
    let _uniform_cpu = vec![0u8; 64 * 1024];

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

// --- Exported entry points ---

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn init() {
    _init();
}

/// Advance one frame. Returns 0 to keep going, 1 when the run budget
/// is reached (native only; wasm32 always returns 0).
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn frame() -> u32 {
    _frame()
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn finalize() {
    _finalize();
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn run() {
    _init();
    for _ in 0..DEFAULT_FRAMES {
        _frame();
    }
    _finalize();
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run() {
    _init();
    while _frame() == 0 {}
    _finalize();
}

// --- Internal implementation ---

fn _init() {
    obs::emit("[seer.boot] entering init()");
    #[cfg(target_arch = "wasm32")]
    {
        gpu_web::init(gpu_web::PowerPreference::Low);
        let status = gpu_web::status();
        obs::emit(&format!("[gpu_web] init kicked; status={status:?}"));
        if status == gpu_web::GpuStatus::Ready {
            let buf = gpu_web::GameBuffer::create(
                64,
                gpu_web::usage::VERTEX | gpu_web::usage::COPY_DST,
                "gpu_web.demo.vertex",
            );
            if let Some(buf) = buf {
                buf.write(&[0u8; 64]);
                obs::emit(&format!(
                    "[gpu_web] demo buffer created + written handle={}",
                    buf.handle()
                ));
            } else {
                obs::emit("[gpu_web] demo buffer create returned null");
            }
        }
    }
    let mut app = App::new();
    app.add_systems(
        Startup,
        (
            setup,
            trees::setup_trees.after(setup),
            campfire::setup_campfire.after(setup),
        ),
    )
    .add_systems(
        Update,
        (
            physics::wander_input,
            physics::advance_player.after(physics::wander_input),
            physics::resolve_collisions.after(physics::advance_player),
            room::world_bounds_clamp.after(physics::resolve_collisions),
            campfire::flicker_fire.after(room::world_bounds_clamp),
            tick.after(campfire::flicker_fire),
            report_player_pos.after(tick),
        ),
    );
    let frames = frame_budget();
    obs::emit(&format!(
        "[seer.boot] Bevy App built, entering update loop for {frames} frames"
    ));
    APP_STATE.with(|c| *c.borrow_mut() = Some(app));

    #[cfg(not(target_arch = "wasm32"))]
    {
        let multi_dir = std::env::var("SEER_MULTI_FRAME_DIR").ok();
        let checkpoints: Vec<u32> = if multi_dir.is_some() {
            vec![frames / 4, frames / 2, 3 * frames / 4, frames]
        } else {
            vec![frames]
        };
        NATIVE_STATE.with(|c| {
            *c.borrow_mut() = Some(NativeRunState {
                budget: frames,
                counter: 0,
                checkpoints,
                snapshots: Vec::new(),
                multi_dir,
            });
        });
    }
}

fn _frame() -> u32 {
    APP_STATE.with(|c| {
        if let Some(app) = c.borrow_mut().as_mut() {
            app.update();
        }
    });

    #[cfg(not(target_arch = "wasm32"))]
    {
        let (do_snapshot, done) = NATIVE_STATE.with(|c| {
            let mut ns_opt = c.borrow_mut();
            let ns = ns_opt.as_mut().expect("NATIVE_STATE not initialized");
            ns.counter += 1;
            let do_snapshot = ns.checkpoints.contains(&ns.counter);
            let done = ns.counter >= ns.budget;
            (do_snapshot, done)
        });
        if do_snapshot {
            let snap = APP_STATE.with(|c| {
                let mut a = c.borrow_mut();
                let app = a.as_mut().expect("APP_STATE not initialized");
                snapshot_scene(app)
            });
            NATIVE_STATE.with(|c| {
                let mut ns_opt = c.borrow_mut();
                let ns = ns_opt.as_mut().unwrap();
                ns.snapshots.push(snap);
            });
        }
        if done { 1 } else { 0 }
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

fn _finalize() {
    #[cfg(not(target_arch = "wasm32"))]
    NATIVE_STATE.with(|c| {
        let ns_opt = c.borrow();
        if let Some(ns) = ns_opt.as_ref() {
            if let Some(dir) = &ns.multi_dir {
                match render_snapshots(&ns.snapshots, dir) {
                    Ok(paths) => obs::emit(&format!(
                        "[gpu.native] multi-frame render: {}",
                        paths.join(", ")
                    )),
                    Err(e) => obs::emit(&format!("[gpu.native] multi-frame render failed: {e}")),
                }
            } else if let Ok(out_path) = std::env::var("SEER_FRAME_PATH")
                && let Some(snap) = ns.snapshots.last()
            {
                match render_single(snap, &out_path) {
                    Ok(_) => obs::emit(&format!("[gpu.native] rendered {out_path}")),
                    Err(e) => obs::emit(&format!("[gpu.native] render failed: {e}")),
                }
            }
        }
    });

    obs::emit("[seer.done] final report:");
    obs::dump_report();
    obs::dump_gpu_inventory();
}

#[cfg(not(target_arch = "wasm32"))]
struct SceneSnapshot {
    trees: Vec<bevy_math::Vec3>,
    obstacles: Vec<bevy_math::Vec3>,
    fires: Vec<(bevy_math::Vec3, f32)>,
    player: bevy_math::Vec3,
}

#[cfg(not(target_arch = "wasm32"))]
fn snapshot_scene(app: &mut App) -> SceneSnapshot {
    let world = app.world_mut();
    let mut tree_q =
        world.query_filtered::<&physics::Position, bevy_ecs::prelude::With<trees::TreeTrunk>>();
    let trees: Vec<bevy_math::Vec3> = tree_q.iter(world).map(|p| p.0).collect();
    let mut obs_q = world.query_filtered::<&physics::Position, (
        bevy_ecs::prelude::With<physics::AabbCollider>,
        bevy_ecs::prelude::Without<physics::PlayerMarker>,
        bevy_ecs::prelude::Without<trees::TreeTrunk>,
        bevy_ecs::prelude::Without<campfire::Campfire>,
    )>();
    let obstacles: Vec<bevy_math::Vec3> = obs_q.iter(world).map(|p| p.0).collect();
    let mut fire_q = world.query::<(&physics::Position, &campfire::Campfire)>();
    let fires: Vec<(bevy_math::Vec3, f32)> = fire_q
        .iter(world)
        .map(|(p, f)| (p.0, f.intensity))
        .collect();
    let mut player_q = world
        .query_filtered::<&physics::Position, bevy_ecs::prelude::With<physics::PlayerMarker>>();
    let player = player_q
        .iter(world)
        .next()
        .map(|p| p.0)
        .unwrap_or(bevy_math::Vec3::ZERO);
    SceneSnapshot {
        trees,
        obstacles,
        fires,
        player,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn snapshot_to_instances(snap: &SceneSnapshot) -> Vec<render::SceneInstance> {
    let floor_half = room::FLOOR_HALF;
    let mut instances: Vec<render::SceneInstance> = Vec::with_capacity(
        1 + snap.trees.len() + snap.obstacles.len() + snap.fires.len() + 1,
    );
    instances.push(render::SceneInstance {
        pos: [0.0, -50.0, 0.0],
        color: [0.09, 0.11, 0.15],
        scale: [floor_half * 2.0, 100.0, floor_half * 2.0],
    });
    for t in &snap.trees {
        instances.push(render::SceneInstance {
            pos: [t.x, 60.0, t.z],
            color: [0.13, 0.77, 0.37],
            scale: [40.0, 130.0, 40.0],
        });
    }
    for o in &snap.obstacles {
        instances.push(render::SceneInstance {
            pos: [o.x, 40.0, o.z],
            color: [0.92, 0.70, 0.03],
            scale: [60.0, 80.0, 60.0],
        });
    }
    for (fire_pos, intensity) in &snap.fires {
        let i = intensity.clamp(0.5, 1.5);
        instances.push(render::SceneInstance {
            pos: [fire_pos.x, 30.0, fire_pos.z],
            color: [1.0 * i, 0.45 * i, 0.08 * i],
            scale: [50.0, 60.0, 50.0],
        });
    }
    instances.push(render::SceneInstance {
        pos: [snap.player.x, 60.0, snap.player.z],
        color: [0.13, 0.83, 0.93],
        scale: [70.0, 140.0, 70.0],
    });
    instances
}

#[cfg(not(target_arch = "wasm32"))]
fn init_wgpu() -> Result<(gpu::SeerDevice, wgpu::Queue), Box<dyn std::error::Error>> {
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
    Ok((gpu::SeerDevice::new(device), queue))
}

#[cfg(not(target_arch = "wasm32"))]
fn render_single(
    snap: &SceneSnapshot,
    out_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (dev, queue) = init_wgpu()?;
    let instances = snapshot_to_instances(snap);
    let camera = render::SceneCamera::follow(
        [snap.player.x, snap.player.y, snap.player.z],
        room::FLOOR_HALF,
    );
    render::render_scene(&dev, &queue, &camera, &instances, out_path)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn render_snapshots(
    snapshots: &[SceneSnapshot],
    dir: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let (dev, queue) = init_wgpu()?;
    let mut out_paths = Vec::with_capacity(snapshots.len());
    for (i, snap) in snapshots.iter().enumerate() {
        let out_path = format!("{dir}/frame-{i}.png");
        let instances = snapshot_to_instances(snap);
        let camera = render::SceneCamera::follow(
            [snap.player.x, snap.player.y, snap.player.z],
            room::FLOOR_HALF,
        );
        render::render_scene(&dev, &queue, &camera, &instances, &out_path)?;
        out_paths.push(out_path);
    }
    Ok(out_paths)
}
