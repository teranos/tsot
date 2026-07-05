// Wasmtime host binary for the seer wasm module.
//
// Founding principle: every wasm→host boundary crossing is a Rust host
// function you own. This host currently provides exactly one import,
// `env.seer_emit(ptr, len)`, which the wasm module calls to route a
// UTF-8 string out. Each call is recorded to an in-memory ledger that
// prints at the end of the run.
//
// This is the dev+diagnostic runtime. The same wasm can later ship to
// the browser with a browser-side JS shim providing `seer_emit`; the
// wasm module itself is unchanged.

use anyhow::{Context, Result, anyhow};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use wasmtime::*;

struct HostState {
    ledger: Vec<String>,
    hotspot_backtraces: BTreeMap<u32, String>,
}

fn main() -> Result<()> {
    let wasm_path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: seer-host <path-to-seer.wasm>"))?;

    println!("[host] engine init");
    let engine = Engine::default();
    println!("[host] loading module: {wasm_path}");
    let module = Module::from_file(&engine, &wasm_path)
        .with_context(|| format!("loading module from {wasm_path}"))?;

    let state = Arc::new(Mutex::new(HostState {
        ledger: Vec::new(),
        hotspot_backtraces: BTreeMap::new(),
    }));
    let mut store: Store<Arc<Mutex<HostState>>> = Store::new(&engine, state.clone());
    let mut linker: Linker<Arc<Mutex<HostState>>> = Linker::new(&engine);

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
                st.ledger.push(format!("seer_emit len={len}: {s}"));
            }
            Ok(())
        },
    )?;

    // Every wasm-side allocation >= 64 KB calls this — the host
    // captures the wasm-side call stack at the boundary and files it
    // under `seq`. Later, when the wasm dumps its hotspot ring, each
    // line carries the seq; correlate with this ledger for the stack.
    // This is the "source attribution from inside the wasm" closed via
    // the wasmtime host — the founding principle made concrete.
    linker.func_wrap(
        "env",
        "seer_record_hotspot",
        |caller: Caller<'_, Arc<Mutex<HostState>>>, seq: u32, size: u32, align: u32| -> Result<()> {
            let bt = WasmBacktrace::force_capture(&caller);
            let mut rendered = String::new();
            for (i, frame) in bt.frames().iter().enumerate() {
                let name = frame.func_name().unwrap_or("<anonymous>");
                let func_idx = frame.func_index();
                rendered.push_str(&format!("  {i:>3}: {name} (func_index={func_idx})\n"));
            }
            let state = caller.data().clone();
            if let Ok(mut st) = state.lock() {
                st.hotspot_backtraces
                    .insert(seq, format!("size={size} align={align}\n{rendered}"));
                st.ledger.push(format!(
                    "seer_record_hotspot seq={seq} size={size} align={align} frames={}",
                    bt.frames().len()
                ));
            }
            Ok(())
        },
    )?;

    println!("[host] instantiating");
    let instance = linker.instantiate(&mut store, &module)?;

    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .context("seer.wasm must export a `run` function")?;

    println!("[host] calling run()");
    run.call(&mut store, ())?;
    println!("[host] run() returned");

    let st = state.lock().map_err(|e| anyhow!("state mutex poisoned: {e}"))?;
    println!(
        "[host.ledger] {} host-function calls recorded during run():",
        st.ledger.len()
    );
    for entry in st.ledger.iter() {
        println!("  {entry}");
    }

    println!(
        "[host.hotspot-backtraces] {} distinct seq entries:",
        st.hotspot_backtraces.len()
    );
    for (seq, bt) in st.hotspot_backtraces.iter() {
        println!("[host.hotspot seq={seq}]");
        println!("{bt}");
    }

    Ok(())
}
