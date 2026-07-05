// Host-side world state accumulated during a run. Every wasm→host
// boundary crossing mutates this via one of the linker.func_wrap
// bodies in imports.rs. Read at end of run by summary + report.

use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub struct Metric {
    pub frame: u32,
    pub heap_bytes: u32,
    pub gpu_live: u32,
    pub gpu_bytes: u32,
}

#[derive(Clone)]
pub struct HotspotRecord {
    pub size: u32,
    pub align: u32,
    pub backtrace: String,
}

#[derive(Clone)]
pub struct GpuRecord {
    pub kind: u32, // 1=buffer 2=texture 3=shader
    pub size: u32,
    pub backtrace: String,
}

pub struct HostState {
    pub ledger: Vec<String>,
    pub hotspot_records: BTreeMap<u32, HotspotRecord>,
    pub gpu_records: BTreeMap<u32, GpuRecord>,
    pub metrics: Vec<Metric>,
    /// Sacred errors captured from the wasm-side bus. Populated by
    /// `seer_emit` when it sees a line starting with "[seer.error".
    /// Report surfaces these prominently so the axiom "errors are
    /// sacred, never dropped" is visibly enforced.
    pub errors_captured: Vec<String>,
}

impl HostState {
    pub fn new() -> Self {
        Self {
            ledger: Vec::new(),
            hotspot_records: BTreeMap::new(),
            gpu_records: BTreeMap::new(),
            metrics: Vec::new(),
            errors_captured: Vec::new(),
        }
    }
}

pub fn kind_name(kind: u32) -> &'static str {
    match kind {
        1 => "buffer",
        2 => "texture",
        3 => "shader",
        _ => "?",
    }
}
