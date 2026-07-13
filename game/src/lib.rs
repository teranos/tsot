// Game entry point. Same schedule on native and wasm; the module
// exports init/frame/finalize as separate wasm functions so a browser
// bootstrap can yield to the event loop between frames. `run()`
// bundles them for the wasmtime/native paths.

use std::cell::RefCell;

pub mod audio;
pub mod build_info;
pub mod campfire;
pub mod campsite;
pub mod cdda;
pub mod chunk;
pub mod dpad;
pub mod error;
pub mod hash;
pub mod health;
pub mod hud;
pub mod identity;
pub mod input;
pub mod jukebox;
pub mod map;
pub mod music;
pub mod net;
pub mod obs;
pub mod palette;
pub mod persist;
pub mod physics;
pub mod remote_players;
pub mod room;
pub mod scene;
pub mod template;
pub mod trail;
pub mod trees;
pub mod ui;

pub mod gpu_web;

#[cfg(target_arch = "wasm32")]
pub mod render_web;

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

use physics::{AabbCollider, NpcMarker, PlayerMarker, Position, Velocity};

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
    snapshots: Vec<scene::SceneSnapshot>,
    multi_dir: Option<String>,
}

#[derive(Resource, Default)]
struct FrameCount(u32);

#[derive(Resource, Default, Clone)]
struct SelfPeer(String);

// The looped track starts playing at the default level. Its handle
// lives in the `music::Music` resource, whose Drop → game_audio_stop
// fires on app teardown. The HUD toggle, the jukebox, and the settings
// slider all drive this one resource.
fn setup_music(mut commands: Commands) {
    let handle = audio::load_music();
    audio::play(&handle, audio::DEFAULT_VOLUME, true);
    commands.insert_resource(music::Music {
        handle,
        playing: true,
        volume: audio::DEFAULT_VOLUME,
    });
}

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
    commands.insert_resource(remote_players::RemotePlayers::default());
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
        // Resume where we left off if a position was saved.
        Position(persist::load().unwrap_or(room::SPAWN_POS)),
        Velocity(Vec3::new(1.5, 0.0, 0.7)),
    ));
    // Circling NPC — same wander pattern as the deterministic native
    // player input; bumping into it fires the exclamation overlay.
    commands.spawn((
        NpcMarker,
        Position(Vec3::new(300.0, 0.0, 300.0)),
        Velocity(Vec3::ZERO),
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

fn publish_self_position_system(
    frame: Res<FrameCount>,
    q: Query<&Position, With<PlayerMarker>>,
    self_peer: Res<SelfPeer>,
) {
    // ~10Hz publish at 60Hz frame budget. Matches rave's 100ms cadence.
    if !frame.0.is_multiple_of(6) {
        return;
    }
    let Ok(p) = q.single() else {
        return;
    };
    if let Err(e) =
        remote_players::publish_position(&self_peer.0, p.0, remote_players::now_ms())
    {
        obs::emit(&format!("[remote_players.publish] error: {e:?}"));
    }
}

fn persist_position_system(
    frame: Res<FrameCount>,
    player_q: Query<&Position, With<PlayerMarker>>,
    structures: Query<(&Position, &template::StructureProp)>,
    npcs: Query<&Position, With<NpcMarker>>,
    mut was_safe_inside: Local<bool>,
) {
    if !frame.0.is_multiple_of(15) {
        return;
    }
    let Ok(player) = player_q.single() else {
        return;
    };
    // "Inside" = a roof tile roughly overhead.
    let inside = structures.iter().any(|(p, s)| {
        s.kind == template::PropKind::Roof
            && (p.0.x - player.0.x).abs() < 80.0
            && (p.0.z - player.0.z).abs() < 80.0
    });
    let enemy_near = npcs.iter().any(|n| (n.0 - player.0).length() < 800.0);
    let safe_inside = inside && !enemy_near;
    // Checkpoint: save on entering a safe, enclosed area.
    if safe_inside && !*was_safe_inside {
        persist::save(player.0);
    }
    *was_safe_inside = safe_inside;
}

fn drain_remote_positions_system(
    mut remotes: ResMut<remote_players::RemotePlayers>,
    self_peer: Res<SelfPeer>,
) {
    let now = remote_players::now_ms();
    if let Err(e) = remote_players::pump_from_proxy(&mut remotes, now, &self_peer.0) {
        obs::emit(&format!("[remote_players.pump] error: {e:?}"));
    }
    remote_players::evict_stale(&mut remotes, now);
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

// The running binary reports its own identity. These expose the
// compile-time build_info (SEER_BUILD_COMMIT / SEER_BUILD_TIME) so the
// JS shim can paint a persistent on-screen badge sourced from THIS
// wasm — not from build-info.json, which is a separate file that can
// skew from the actual binary. "What version is running" then has one
// unambiguous answer: what the wasm says about itself.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn build_commit_ptr() -> *const u8 {
    build_info::COMMIT.as_ptr()
}
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn build_commit_len() -> u32 {
    build_info::COMMIT.len() as u32
}
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn build_time_ptr() -> *const u8 {
    build_info::BUILT_AT.as_ptr()
}
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn build_time_len() -> u32 {
    build_info::BUILT_AT.len() as u32
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
    let id = identity::Identity::load_or_create();
    obs::emit(&format!(
        "[identity] {} ({})",
        id.short(),
        if id.is_new { "new" } else { "loaded" }
    ));
    #[cfg(target_arch = "wasm32")]
    {
        gpu_web::init(gpu_web::PowerPreference::Low);
        let status = gpu_web::status();
        obs::emit(&format!("[gpu_web] init kicked; status={status:?}"));
        if status == gpu_web::GpuStatus::Ready {
            render_web::init("#game-canvas");
        }
    }
    let mut app = App::new();
    app.insert_resource(SelfPeer(id.as_hex()));
    app.insert_resource(chunk::LoadedChunks::default());
    app.insert_resource(cdda::load_building_templates());
    app.add_systems(
        Startup,
        (
            setup,
            campfire::setup_campfire.after(setup),
            dpad::setup_dpad.after(setup),
            hud::setup_hud.after(setup),
            jukebox::setup_jukebox.after(setup),
            map::setup_pins.after(setup),
            trail::setup_trail.after(setup),
            setup_music.after(setup),
        ),
    );
    #[cfg(target_arch = "wasm32")]
    let input_system = physics::keyboard_input;
    #[cfg(not(target_arch = "wasm32"))]
    let input_system = physics::wander_input;
    app.add_systems(
        Update,
        (
            input_system,
            physics::wander_npc,
            physics::advance_player.after(input_system),
            physics::advance_npc.after(physics::wander_npc),
            physics::resolve_collisions.after(physics::advance_player),
            physics::resolve_remote_player_collisions.after(physics::resolve_collisions),
            physics::check_npc_bump.after(physics::advance_npc),
            chunk::stream_chunks.after(physics::resolve_remote_player_collisions),
            campfire::flicker_fire.after(physics::resolve_remote_player_collisions),
            campfire::campfire_crackle_system.after(campfire::flicker_fire),
            dpad::dpad_input_system.after(campfire::campfire_crackle_system),
            hud::hud_input_system.after(dpad::dpad_input_system),
            jukebox::jukebox_proximity_system.after(physics::resolve_collisions),
            tick.after(campfire::flicker_fire),
            drain_remote_positions_system.after(tick),
            publish_self_position_system.after(physics::advance_player),
            persist_position_system.after(physics::advance_player),
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

    #[cfg(target_arch = "wasm32")]
    APP_STATE.with(|c| {
        if let Some(app) = c.borrow_mut().as_mut() {
            let _ = render_web::frame_from_app(app);
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
                scene::snapshot_scene(app)
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
    snap: &scene::SceneSnapshot,
    out_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (dev, queue) = init_wgpu()?;
    let instances = scene::snapshot_to_instances(snap);
    let glass = scene::snapshot_to_glass_instances(snap);
    let camera = scene::SceneCamera::follow(
        [snap.player.x, snap.player.y, snap.player.z],
        room::FLOOR_HALF,
    );
    render::render_scene(&dev, &queue, &camera, &instances, &glass, out_path)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn render_snapshots(
    snapshots: &[scene::SceneSnapshot],
    dir: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let (dev, queue) = init_wgpu()?;
    let mut out_paths = Vec::with_capacity(snapshots.len());
    for (i, snap) in snapshots.iter().enumerate() {
        let out_path = format!("{dir}/frame-{i}.png");
        let instances = scene::snapshot_to_instances(snap);
        let glass = scene::snapshot_to_glass_instances(snap);
        let camera = scene::SceneCamera::follow(
            [snap.player.x, snap.player.y, snap.player.z],
            room::FLOOR_HALF,
        );
        render::render_scene(&dev, &queue, &camera, &instances, &glass, &out_path)?;
        out_paths.push(out_path);
    }
    Ok(out_paths)
}
