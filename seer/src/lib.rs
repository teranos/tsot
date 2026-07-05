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

    // Simulated frame loop with a grow-over-time pattern that stands in
    // for a real rendering workload with a slow leak. Sizes chosen to
    // resemble the categories observed in rave sessions:
    //   - Cluster storage buffer (~200 KB) replaced every frame (churn)
    //   - Uniform buffer (~64 KB) replaced every frame (churn)
    //   - Mesh instance chunk (~512 KB) retained every 5th frame
    //   - Scene texture buffer (~1 MB) retained every 10th frame
    // The retained allocations model a system that fails to release
    // resources — RSS keeps rising while churn stays flat.
    //
    // Real Bevy + wgpu wiring lands in commit #2. Until then, this
    // stresses the obs bus with a workload it can actually reason
    // about, and gives seer-host a real ledger of many boundary
    // crossings to record.
    let mut retained: Vec<Vec<u8>> = Vec::new();
    const FRAMES: u32 = 300;
    const REPORT_EVERY: u32 = 30;

    for frame in 0..FRAMES {
        let _cluster = vec![0u8; 200 * 1024];
        let _uniform = vec![0u8; 64 * 1024];

        if frame % 5 == 0 {
            retained.push(vec![0u8; 512 * 1024]);
        }
        if frame % 10 == 0 {
            retained.push(vec![0u8; 1024 * 1024]);
        }

        if frame % REPORT_EVERY == 0 {
            obs::emit(&format!(
                "[seer.frame] frame={frame} retained_bufs={}",
                retained.len()
            ));
            obs::dump_report();
        }
    }

    obs::emit("[seer.done] final report:");
    obs::dump_report();
    obs::emit(&format!(
        "[seer.done] retained {} buffers over {FRAMES} frames",
        retained.len()
    ));
}
