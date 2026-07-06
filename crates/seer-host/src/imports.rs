// Wasm-boundary surface. All four env.* imports the wasm module
// expects live here. If a new one lands in seer/src/obs.rs, wire it
// in wire_imports — the seer.wasm module fails to instantiate
// otherwise, by design.

use anyhow::{Result, anyhow};
use rustc_demangle::demangle;
use std::sync::{Arc, Mutex};
use wasmtime::*;

use crate::state::{GpuRecord, HostState, HotspotRecord, Metric, kind_name};

pub fn wire_imports(linker: &mut Linker<Arc<Mutex<HostState>>>) -> Result<()> {
    linker.func_wrap(
        "env",
        "seer_emit",
        |mut caller: Caller<'_, Arc<Mutex<HostState>>>, ptr: i32, len: i32| -> Result<()> {
            let memory = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| anyhow!("wasm module has no 'memory' export"))?;
            let mut buf = vec![0u8; len as usize];
            memory.read(&caller, ptr as usize, &mut buf)?;
            let s = String::from_utf8_lossy(&buf).into_owned();
            println!("[host.emit] {s}");
            let state = caller.data().clone();
            if let Ok(mut st) = state.lock() {
                if s.starts_with("[seer.error") {
                    st.errors_captured.push(s.clone());
                }
                st.ledger.push(format!("seer_emit len={len}: {s}"));
            }
            Ok(())
        },
    )?;

    // Every wasm-side allocation >= 64 KB calls this — the host
    // captures the wasm call stack at the boundary and files it under
    // `seq`. Later, when the wasm dumps its hotspot ring, each line
    // carries the seq; correlate with this ledger for the stack.
    linker.func_wrap(
        "env",
        "seer_record_hotspot",
        |caller: Caller<'_, Arc<Mutex<HostState>>>, seq: u32, size: u32, align: u32| -> Result<()> {
            let bt = WasmBacktrace::force_capture(&caller);
            let backtrace = render_wasm_backtrace(&bt);
            let frames_len = bt.frames().len();
            let state = caller.data().clone();
            if let Ok(mut st) = state.lock() {
                st.hotspot_records.insert(
                    seq,
                    HotspotRecord {
                        size,
                        align,
                        backtrace,
                    },
                );
                st.ledger.push(format!(
                    "seer_record_hotspot seq={seq} size={size} align={align} frames={frames_len}"
                ));
            }
            Ok(())
        },
    )?;

    // Same host-ledger pattern, keyed by gpu id, partitioned by kind
    // so seq spaces don't collide. `label_ptr`/`label_len` (added for
    // Task 6) point into wasm memory at the resource name; decoded
    // utf-8 lands in `GpuRecord.label`.
    linker.func_wrap(
        "env",
        "seer_record_gpu_event",
        |mut caller: Caller<'_, Arc<Mutex<HostState>>>,
         id: u32,
         kind: u32,
         size: u32,
         label_ptr: i32,
         label_len: i32|
         -> Result<()> {
            let memory = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| anyhow!("wasm module has no 'memory' export"))?;
            let mut buf = vec![0u8; label_len as usize];
            memory.read(&caller, label_ptr as usize, &mut buf)?;
            let label = String::from_utf8_lossy(&buf).into_owned();

            let bt = WasmBacktrace::force_capture(&caller);
            let backtrace = render_wasm_backtrace(&bt);
            let frames_len = bt.frames().len();
            let kname = kind_name(kind);
            let state = caller.data().clone();
            if let Ok(mut st) = state.lock() {
                let created_at_seq = st.ledger.len();
                st.gpu_records.insert(
                    id,
                    GpuRecord {
                        kind,
                        size,
                        backtrace,
                        label: label.clone(),
                        created_at_seq,
                        destroyed_at_seq: None,
                    },
                );
                st.ledger.push(format!(
                    "seer_record_gpu_event id={id} kind={kname} size={size} label={label:?} frames={frames_len}"
                ));
            }
            Ok(())
        },
    )?;

    // Destroy counterpart (Task 7). No backtrace here — the destroy
    // site is less interesting than the create site, and skipping
    // WasmBacktrace keeps the boundary crossing cheap. Pairs with
    // the earlier gpu_records entry to fill in destroyed_at_seq.
    linker.func_wrap(
        "env",
        "seer_record_gpu_destroyed",
        |caller: Caller<'_, Arc<Mutex<HostState>>>, id: u32| -> Result<()> {
            let state = caller.data().clone();
            if let Ok(mut st) = state.lock() {
                let destroyed_at_seq = st.ledger.len();
                if let Some(rec) = st.gpu_records.get_mut(&id) {
                    rec.destroyed_at_seq = Some(destroyed_at_seq);
                }
                st.ledger
                    .push(format!("seer_record_gpu_destroyed id={id}"));
            }
            Ok(())
        },
    )?;

    // Structured per-frame metric. Cheap: no backtrace capture, just
    // four numbers. Feeds the HTML time-series chart.
    linker.func_wrap(
        "env",
        "seer_report_metric",
        |caller: Caller<'_, Arc<Mutex<HostState>>>,
         frame: u32,
         heap_bytes: u32,
         gpu_live: u32,
         gpu_bytes: u32|
         -> Result<()> {
            let state = caller.data().clone();
            if let Ok(mut st) = state.lock() {
                st.metrics.push(Metric {
                    frame,
                    heap_bytes,
                    gpu_live,
                    gpu_bytes,
                });
            }
            Ok(())
        },
    )?;

    Ok(())
}

fn render_wasm_backtrace(bt: &WasmBacktrace) -> String {
    let mut out = String::new();
    for (i, frame) in bt.frames().iter().enumerate() {
        let name = frame.func_name().unwrap_or("<anonymous>");
        let demangled = demangle(name);
        let func_idx = frame.func_index();
        out.push_str(&format!("  {i:>3}: {demangled:#} (func_index={func_idx})\n"));
    }
    out
}
