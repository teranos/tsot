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
    // Simulated one-time shader compile at startup.
    let sid = obs::shader_created(4096, "seer.pbr");
    obs::emit(&format!(
        "[seer.setup] created shader id={sid} for demo — stays live forever"
    ));
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

fn _run() {
    obs::emit("[seer.boot] entering run()");
    let mut app = App::new();
    app.add_systems(Startup, setup)
        .add_systems(Update, tick);
    obs::emit("[seer.boot] Bevy App built, entering update loop");
    for _ in 0..FRAMES {
        app.update();
    }
    obs::emit("[seer.done] final report:");
    obs::dump_report();
}
