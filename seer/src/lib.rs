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

fn _run() {
    obs::emit("[seer.boot] entering run()");

    // Foundation-commit workload: allocate a small variety of buffer
    // sizes to exercise the aggregate counters AND the per-alloc
    // hotspot ring. In commit #2+ this becomes actual game work
    // (Bevy ECS, wgpu resources).
    let mut retained: Vec<Vec<u8>> = Vec::new();
    for i in 0..5 {
        let size = 128 * 1024 * (i + 1);
        obs::emit(&format!("[seer.step] allocating {size} bytes"));
        retained.push(vec![0u8; size]);
    }

    obs::dump_report();
    obs::emit(&format!(
        "[seer.done] retained {} buffers, exiting",
        retained.len()
    ));
}
