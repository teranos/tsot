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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
            let source = capture_source();
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
fn capture_source() -> String {
    std::backtrace::Backtrace::force_capture().to_string()
}

#[cfg(target_arch = "wasm32")]
fn capture_source() -> String {
    String::from("<wasm: source attribution via seer-host boundary ledger>")
}

pub fn emit(line: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        println!("{}", line);
    }
    #[cfg(target_arch = "wasm32")]
    unsafe {
        seer_emit(line.as_ptr(), line.len());
    }
}

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn seer_emit(ptr: *const u8, len: usize);
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
