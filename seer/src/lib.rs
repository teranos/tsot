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

pub mod obs;
pub mod physics;

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
    // Retained handles — the "leak" pattern for GPU resources.
    retained: Vec<u64>,
}

const FRAMES: u32 = 300;
const REPORT_EVERY: u32 = 30;

fn setup(mut commands: Commands) {
    obs::emit("[bevy.setup] Startup schedule running");
    commands.insert_resource(FrameCount::default());
    commands.insert_resource(Retained::default());
    commands.insert_resource(GpuHandles::default());
    let sid = obs::shader_created(4096, "seer.pbr");
    obs::emit(&format!(
        "[seer.setup] created shader id={sid} for demo — stays live forever"
    ));

    // Ported from rave: spawn a player + 5 static obstacles that the
    // resolve_collisions system iterates every frame. Real ECS query
    // pattern with With<PlayerMarker> / Without<PlayerMarker> filters.
    commands.spawn((
        PlayerMarker,
        Position(Vec3::new(0.0, 0.0, 0.0)),
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
    mut retained: ResMut<Retained>,
    mut gpu: ResMut<GpuHandles>,
) {
    count.0 += 1;
    let frame = count.0;

    // Per-frame heap churn.
    let _cluster_cpu = vec![0u8; 200 * 1024];
    let _uniform_cpu = vec![0u8; 64 * 1024];

    // GPU resources: every frame allocates a cluster storage buffer +
    // uniform buffer. We DESTROY the previous frame's cluster (churn)
    // but retain the uniform (leak pattern — the exact rave signature).
    for id in gpu.cluster.drain(..) {
        obs::resource_destroyed(id);
    }
    let cluster_id = obs::buffer_created(200 * 1024, 0x80 /* STORAGE */, "GpuClusterableObjectIndexListsStorage");
    let uniform_id = obs::buffer_created(64 * 1024, 0x40 /* UNIFORM */, "GpuGlobalsBuffer");
    gpu.cluster.push(cluster_id);
    gpu.retained.push(uniform_id);

    // Retained CPU heap allocations (Phase 2 pattern retained).
    if frame.is_multiple_of(5) {
        retained.0.push(vec![0u8; 512 * 1024]);
    }
    if frame.is_multiple_of(10) {
        retained.0.push(vec![0u8; 1024 * 1024]);
        // Every 10 frames also create a retained GPU texture (real scene texture pattern).
        let tid = obs::texture_created(1024 * 1024, 0x04 /* SAMPLED */, "scene.diffuse");
        gpu.retained.push(tid);
    }

    if frame.is_multiple_of(REPORT_EVERY) {
        obs::emit(&format!(
            "[bevy.tick] frame={frame} retained_cpu_bufs={} retained_gpu_handles={}",
            retained.0.len(),
            gpu.retained.len(),
        ));
        obs::dump_report();
        obs::dump_gpu_inventory();
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
    app.add_systems(Startup, setup).add_systems(
        Update,
        (
            physics::advance_player,
            physics::resolve_collisions.after(physics::advance_player),
            tick.after(physics::resolve_collisions),
            report_player_pos.after(tick),
        ),
    );
    obs::emit("[seer.boot] Bevy App built, entering update loop");
    for _ in 0..FRAMES {
        app.update();
    }

    #[cfg(not(target_arch = "wasm32"))]
    native_wgpu_demo();

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

    let (device, _queue) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
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
    obs::emit("[gpu.native] dropped all real buffers — SeerBuffer::drop should have emitted destroyed events:");
    obs::dump_gpu_inventory();
}
