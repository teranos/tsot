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

pub struct HostState {
    pub ledger: Vec<String>,
    pub hotspot_backtraces: BTreeMap<u32, String>,
    pub gpu_backtraces: BTreeMap<u32, String>,
    pub metrics: Vec<Metric>,
}

impl HostState {
    pub fn new() -> Self {
        Self {
            ledger: Vec::new(),
            hotspot_backtraces: BTreeMap::new(),
            gpu_backtraces: BTreeMap::new(),
            metrics: Vec::new(),
        }
    }
}
