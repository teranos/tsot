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

const FRAMES: u32 = 300;
const REPORT_EVERY: u32 = 30;

fn setup(mut commands: Commands) {
    obs::emit("[bevy.setup] Startup schedule running");
    commands.insert_resource(FrameCount::default());
    commands.insert_resource(Retained::default());
}

fn tick(mut count: ResMut<FrameCount>, mut retained: ResMut<Retained>) {
    count.0 += 1;
    let frame = count.0;

    // Per-frame churn — replaces would be here in a real render system.
    // These are dropped at the end of the system.
    let _cluster = vec![0u8; 200 * 1024];
    let _uniform = vec![0u8; 64 * 1024];

    // Retained-over-time — the "leak" pattern.
    if frame.is_multiple_of(5) {
        retained.0.push(vec![0u8; 512 * 1024]);
    }
    if frame.is_multiple_of(10) {
        retained.0.push(vec![0u8; 1024 * 1024]);
    }

    if frame.is_multiple_of(REPORT_EVERY) {
        obs::emit(&format!(
            "[bevy.tick] frame={frame} retained_bufs={}",
            retained.0.len()
        ));
        obs::dump_report();
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
