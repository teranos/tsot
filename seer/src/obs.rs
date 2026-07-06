// The observability bus. Founding module. Every allocation, every
// deliberate event, flows through here. Same code on native and wasm.
//
// Two layers of coverage:
//   1. Aggregate counters (bytes/peak/count). Cheap, always on, both
//      targets. Wire: `ALLOC_BYTES`, `ALLOC_PEAK`, `ALLOC_COUNT`.
//   2. Per-large-allocation source capture (>= HOTSPOT_THRESHOLD).
//      Bounded ring. Reentrancy-guarded. On native, `Backtrace::force_capture`
//      gives Rust source paths (release profile has `debug = 1`). On wasm,
//      per-alloc source capture inside the allocator is limited without
//      an unwind mechanism — that gap is closed by the host: `seer-host`
//      records every `seer_emit` call it observes, so caller-attribution
//      lives on the host side of the boundary.
//
// Sink: `emit(line)` writes to stdout on native, to the imported extern
// `seer_emit` on wasm. `seer-host` is the wasmtime binary that provides
// that extern and prints what the wasm emits.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);
pub static ALLOC_PEAK: AtomicUsize = AtomicUsize::new(0);
pub static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

static IN_HEAVY_ALLOC: AtomicBool = AtomicBool::new(false);
static HOTSPOTS: Mutex<Vec<Hotspot>> = Mutex::new(Vec::new());

pub const HOTSPOT_THRESHOLD: usize = 65_536;
pub const HOTSPOT_CAPACITY: usize = 128;

pub struct Hotspot {
    pub seq: usize,
    pub size: usize,
    pub align: usize,
    pub source: String,
}

pub struct InstrumentedAlloc;

unsafe impl GlobalAlloc for InstrumentedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let n = ALLOC_BYTES.fetch_add(size, Ordering::Relaxed) + size;
        let seq = ALLOC_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        let mut peak = ALLOC_PEAK.load(Ordering::Relaxed);
        while n > peak {
            match ALLOC_PEAK.compare_exchange_weak(
                peak,
                n,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => peak = x,
            }
        }
        if size >= HOTSPOT_THRESHOLD && !IN_HEAVY_ALLOC.swap(true, Ordering::Relaxed) {
            let source = capture_source_for(seq, size, layout.align());
            if let Ok(mut hs) = HOTSPOTS.lock() {
                if hs.len() >= HOTSPOT_CAPACITY {
                    hs.remove(0);
                }
                hs.push(Hotspot {
                    seq,
                    size,
                    align: layout.align(),
                    source,
                });
            }
            IN_HEAVY_ALLOC.store(false, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOC_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn capture_source_for(_seq: usize, _size: usize, _align: usize) -> String {
    // std::backtrace::Backtrace::force_capture().to_string() does full
    // DWARF symbolication per frame — running that from a GlobalAlloc
    // hook thousands of times per Bevy frame hangs the process
    // indefinitely (verified: 30-frame run doesn't complete in 60s
    // locally, 10-min CI timeout). The native binary's role is fast
    // ECS iteration; source attribution lives in the wasmtime-host
    // path (Phase 1: seer_record_hotspot -> WasmBacktrace) which is
    // essentially free because wasmtime tracks the wasm stack anyway.
    String::new()
}

#[cfg(target_arch = "wasm32")]
fn capture_source_for(seq: usize, size: usize, align: usize) -> String {
    // Wasm has no unwind info; the wasm-side hotspot ring cannot
    // capture a Rust backtrace itself. Instead the wasm calls a host
    // import at this exact site — `seer_record_hotspot` — and the host
    // (seer-host) captures a `wasmtime::WasmBacktrace` at that
    // boundary crossing, keyed by seq. The wasm's own record here just
    // carries the seq; the host ledger has the wasm-side stack. This
    // is what "wasmtime as the primary diagnostic environment" buys.
    unsafe {
        seer_record_hotspot(seq as u32, size as u32, align as u32);
    }
    format!("<host-ledger seq={seq}>")
}

pub fn emit(line: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
    #[cfg(target_arch = "wasm32")]
    unsafe {
        seer_emit(line.as_ptr(), line.len());
    }
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn seer_emit(ptr: *const u8, len: usize);
    fn seer_record_hotspot(seq: u32, size: u32, align: u32);
    // Widened for Task 6: the label carries the resource name across
    // the boundary. Previously the label lived wasm-side in
    // GpuLiveResource and never crossed to the host; now the host's
    // CommitReport gets it too so per-label aggregation is possible
    // report-side.
    fn seer_record_gpu_event(id: u32, kind: u32, size: u32, label_ptr: *const u8, label_len: usize);
    // Added for Task 7: destroy events cross the boundary too, so
    // the host can compute per-resource lifetimes for the histogram.
    fn seer_record_gpu_destroyed(id: u32);
    fn seer_report_metric(frame: u32, heap_bytes: u32, gpu_live: u32, gpu_bytes: u32);
}

/// Structured metric snapshot for the HTML report artifact.
/// Host collects these into a time series and renders a chart.
pub fn emit_metric(frame: u32, heap_bytes: u64, gpu_live: u32, gpu_bytes: u64) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        seer_report_metric(
            frame,
            heap_bytes.min(u32::MAX as u64) as u32,
            gpu_live,
            gpu_bytes.min(u32::MAX as u64) as u32,
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        emit(&format!(
            "[obs.metric] frame={frame} heap={heap_bytes} gpu_live={gpu_live} gpu_bytes={gpu_bytes}"
        ));
    }
}

pub fn gpu_totals() -> (u32, u64) {
    let live = match GPU_LIVE.lock() {
        Ok(l) => l,
        Err(_) => return (0, 0),
    };
    let bytes: u64 = live.iter().map(|r| r.size).sum();
    (live.len() as u32, bytes)
}

// ============================================================
// GPU resource events. Founding wgpu wrapper contract:
// every buffer/texture/shader creation calls one of these, every
// destroy calls resource_destroyed. Live inventory kept here so at
// any moment the obs bus can name every unreleased GPU resource.
// Real wgpu wrapper (Phase 4) plugs into this — the interface is
// deliberately defined ahead of the caller so the wrapper is a
// mechanical adapter, not a design decision.
// ============================================================

#[derive(Clone, Copy)]
pub enum GpuResourceKind {
    Buffer = 1,
    Texture = 2,
    Shader = 3,
}

pub struct GpuLiveResource {
    pub id: u64,
    pub kind: GpuResourceKind,
    pub size: u64,
    pub usage: u32,
    pub label: String,
    pub created_at_alloc_seq: usize,
    pub source: String,
}

static NEXT_GPU_ID: AtomicU64 = AtomicU64::new(0);
static GPU_LIVE: Mutex<Vec<GpuLiveResource>> = Mutex::new(Vec::new());

pub fn buffer_created(size: u64, usage: u32, label: &str) -> u64 {
    resource_created(GpuResourceKind::Buffer, size, usage, label)
}
pub fn texture_created(size: u64, usage: u32, label: &str) -> u64 {
    resource_created(GpuResourceKind::Texture, size, usage, label)
}
pub fn shader_created(code_len: u64, label: &str) -> u64 {
    resource_created(GpuResourceKind::Shader, code_len, 0, label)
}

fn resource_created(kind: GpuResourceKind, size: u64, usage: u32, label: &str) -> u64 {
    let id = NEXT_GPU_ID.fetch_add(1, Ordering::Relaxed) + 1;
    let created_at_alloc_seq = ALLOC_COUNT.load(Ordering::Relaxed);
    let source = capture_gpu_source(id, kind, size, label);
    if let Ok(mut live) = GPU_LIVE.lock() {
        live.push(GpuLiveResource {
            id,
            kind,
            size,
            usage,
            label: label.to_string(),
            created_at_alloc_seq,
            source,
        });
    }
    id
}

pub fn resource_destroyed(id: u64) {
    if let Ok(mut live) = GPU_LIVE.lock() {
        live.retain(|r| r.id != id);
    }
    // Boundary crossing so the host can pair this destroy with its
    // earlier create and compute a lifetime for the report's
    // histogram. Native has no host — the destroy is purely local.
    #[cfg(target_arch = "wasm32")]
    unsafe {
        seer_record_gpu_destroyed(id as u32);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn capture_gpu_source(_id: u64, _kind: GpuResourceKind, _size: u64, _label: &str) -> String {
    // Same reasoning as capture_source_for: native full backtraces
    // are prohibitively expensive from ECS hot paths. Wasmtime host
    // captures WasmBacktrace via seer_record_gpu_event at effectively
    // zero cost; that's the diagnostic channel.
    String::new()
}

#[cfg(target_arch = "wasm32")]
fn capture_gpu_source(id: u64, kind: GpuResourceKind, size: u64, label: &str) -> String {
    // Same host-ledger pattern as heap hotspots: wasm calls, host
    // captures WasmBacktrace under this id. Different import so the
    // host can partition its ledger by event type. The label crosses
    // the boundary so the host's per-label aggregation matches the
    // wasm-side GpuLiveResource.label without a second lookup.
    unsafe {
        seer_record_gpu_event(
            id as u32,
            kind as u32,
            size as u32,
            label.as_ptr(),
            label.len(),
        );
    }
    format!("<host-ledger gpu_id={id}>")
}

pub fn dump_gpu_inventory() {
    let live = match GPU_LIVE.lock() {
        Ok(l) => l,
        Err(_) => {
            emit("[obs.gpu] inventory mutex poisoned");
            return;
        }
    };
    if live.is_empty() {
        emit("[obs.gpu.inventory] live=0");
        return;
    }
    let mut total: u64 = 0;
    let mut by_kind: [(u64, u64); 3] = [(0, 0); 3]; // (count, bytes) for Buffer/Texture/Shader
    for r in live.iter() {
        total += r.size;
        let idx = match r.kind {
            GpuResourceKind::Buffer => 0,
            GpuResourceKind::Texture => 1,
            GpuResourceKind::Shader => 2,
        };
        by_kind[idx].0 += 1;
        by_kind[idx].1 += r.size;
    }
    emit(&format!(
        "[obs.gpu.inventory] live={} total={:.3}MB · buffers={}/{:.3}MB · textures={}/{:.3}MB · shaders={}/{:.3}MB",
        live.len(),
        total as f64 / 1_048_576.0,
        by_kind[0].0, by_kind[0].1 as f64 / 1_048_576.0,
        by_kind[1].0, by_kind[1].1 as f64 / 1_048_576.0,
        by_kind[2].0, by_kind[2].1 as f64 / 1_048_576.0,
    ));
    let mut sorted: Vec<&GpuLiveResource> = live.iter().collect();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.size));
    for r in sorted.iter().take(15) {
        let kind = match r.kind {
            GpuResourceKind::Buffer => "buffer",
            GpuResourceKind::Texture => "texture",
            GpuResourceKind::Shader => "shader",
        };
        emit(&format!(
            "[obs.gpu.live] #{} kind={kind} size={:.3}MB usage=0x{:x} label=\"{}\" created_at_alloc_seq={} source:\n{}",
            r.id,
            r.size as f64 / 1_048_576.0,
            r.usage,
            r.label,
            r.created_at_alloc_seq,
            r.source,
        ));
    }
}

pub fn dump_report() {
    let bytes = ALLOC_BYTES.load(Ordering::Relaxed);
    let peak = ALLOC_PEAK.load(Ordering::Relaxed);
    let count = ALLOC_COUNT.load(Ordering::Relaxed);
    emit(&format!(
        "[obs.summary] bytes={:.2}MB peak={:.2}MB count={}",
        bytes as f64 / 1_048_576.0,
        peak as f64 / 1_048_576.0,
        count,
    ));
    let hs = match HOTSPOTS.lock() {
        Ok(hs) => hs,
        Err(_) => {
            emit("[obs.hotspots] mutex poisoned");
            return;
        }
    };
    if hs.is_empty() {
        emit(&format!(
            "[obs.hotspots] none (no allocations >= {} bytes)",
            HOTSPOT_THRESHOLD
        ));
        return;
    }
    let mut sorted: Vec<&Hotspot> = hs.iter().collect();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.size));
    let top_n = sorted.len().min(10);
    emit(&format!(
        "[obs.hotspots] captured={} top {top_n} by size:",
        hs.len(),
    ));
    for r in sorted.iter().take(top_n) {
        let mb = r.size as f64 / 1_048_576.0;
        emit(&format!(
            "[obs.hotspot] seq={} size={mb:.3}MB align={} source:\n{}",
            r.seq, r.align, r.source,
        ));
    }
}
