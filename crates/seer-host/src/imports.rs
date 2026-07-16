// Wasm-boundary surface. All env.* imports the game module expects
// live here. If a new one lands in crates/seer-obs, wire it in
// wire_imports — game.wasm fails to instantiate otherwise.

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

    // `label_ptr`/`label_len` point into wasm memory at the resource
    // name; decoded utf-8 lands in `GpuRecord.label`.
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

    // No backtrace on destroy — the site is uninteresting and
    // skipping WasmBacktrace keeps the boundary crossing cheap.
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

    // WebGPU init under wasmtime — no browser, no GPU. init is a
    // no-op; status pins to Unavailable so the wasm render path skips.
    linker.func_wrap(
        "env",
        "game_gpu_init",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _power_pref: u32| -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_status",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>| -> Result<u32> { Ok(2) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_buffer_create",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _size: u32,
         _usage: u32,
         _label_ptr: i32,
         _label_len: i32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_buffer_write",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _handle: u32,
         _data_ptr: i32,
         _data_len: i32|
         -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_buffer_destroy",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _handle: u32| -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_shader_module_create",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _src_ptr: i32,
         _src_len: i32,
         _label_ptr: i32,
         _label_len: i32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_bind_group_layout_create_uniform",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _label_ptr: i32,
         _label_len: i32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_bind_group_create",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _layout: u32,
         _buffer: u32,
         _label_ptr: i32,
         _label_len: i32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_pipeline_layout_create",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _bg_layout: u32,
         _label_ptr: i32,
         _label_len: i32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_render_pipeline_create_cube",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _pipeline_layout: u32,
         _shader: u32,
         _vertex_stride: u32,
         _instance_stride: u32,
         _color_format: u32,
         _depth_format: u32,
         _label_ptr: i32,
         _label_len: i32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_render_target_configure",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _canvas_id_ptr: i32,
         _canvas_id_len: i32,
         _color_format: u32,
         _depth_format: u32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_render_frame",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _target: u32,
         _pipeline: u32,
         _bind_group: u32,
         _vertex_buf: u32,
         _instance_buf: u32,
         _vertex_count: u32,
         _instance_count: u32,
         _clear_r: f32,
         _clear_g: f32,
         _clear_b: f32|
         -> Result<u32> { Ok(1) },
    )?;
    linker.func_wrap(
        "env",
        "game_input_state",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>| -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_show_exclamation",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _x: f32, _y: f32| -> Result<()> { Ok(()) },
    )?;
    // Under wasmtime there's no IndexedDB, no user session, no
    // browser crypto. Return "not found" from load so Rust generates
    // an identity via random_bytes; that path stays deterministic
    // because random_bytes here fills with zeros.
    linker.func_wrap(
        "env",
        "game_identity_load",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _out_ptr: i32| -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_identity_save",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _bytes_ptr: i32, _bytes_len: i32|
         -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_random_bytes",
        |mut caller: Caller<'_, Arc<Mutex<HostState>>>, out_ptr: i32, out_len: i32| -> Result<()> {
            let memory = caller
                .get_export("memory")
                .and_then(|e| e.into_memory())
                .ok_or_else(|| anyhow!("wasm module has no 'memory' export"))?;
            let zeros = vec![0u8; out_len as usize];
            memory.write(&mut caller, out_ptr as usize, &zeros)?;
            Ok(())
        },
    )?;
    // No proxy in wasmtime. Pending is always 0, publish is a no-op,
    // now_ms uses the host wall clock so RavePosition.at_ms stays
    // realistic for downstream diagnostics.
    linker.func_wrap(
        "env",
        "game_peers_pending",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>| -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_peers_recv",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _out_ptr: i32, _out_len: i32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_self_publish",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _bytes_ptr: i32, _bytes_len: i32|
         -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_now_ms",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>| -> Result<f64> {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as f64)
                .unwrap_or(0.0);
            Ok(ms)
        },
    )?;
    // No AudioContext under wasmtime — load returns 0 so the Rust
    // GameAudioHandle stays inert, play/stop no-op.
    linker.func_wrap(
        "env",
        "game_audio_load",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _path_ptr: i32, _path_len: i32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_audio_play",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _h: u32, _vol: u32, _loop_flag: u32|
         -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_audio_stop",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _h: u32| -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_audio_play_samples",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _ptr: u32, _count: u32, _rate: u32|
         -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_touch_state",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _out_ptr: u32, _out_max: u32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_viewport_size",
        |mut caller: Caller<'_, Arc<Mutex<HostState>>>, out_ptr: u32| -> Result<()> {
            // Write a plausible viewport for wasmtime host: 1920x1080.
            let mem = caller.get_export("memory").and_then(|e| e.into_memory());
            if let Some(mem) = mem {
                let data = mem.data_mut(&mut caller);
                let width: u32 = 1920;
                let height: u32 = 1080;
                let base = out_ptr as usize;
                if base + 8 <= data.len() {
                    data[base..base + 4].copy_from_slice(&width.to_le_bytes());
                    data[base + 4..base + 8].copy_from_slice(&height.to_le_bytes());
                }
            }
            Ok(())
        },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_render_pipeline_create_ui",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _pl: u32, _shader: u32, _instance_stride: u32,
         _color_format: u32, _label_ptr: u32, _label_len: u32|
         -> Result<u32> { Ok(1) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_render_ui_overlay",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _target: u32, _pipeline: u32, _bind_group: u32,
         _instance_buf: u32, _instance_count: u32|
         -> Result<u32> { Ok(0) },
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

    // Glass + ghost render passes. Same no-GPU treatment as the cube/UI
    // pipelines above: pipeline-create hands back a fake handle, the
    // draw call is inert. These landed on this branch; without them
    // game.wasm won't instantiate under wasmtime (see the imports.allow
    // conformance test at the bottom of this file).
    linker.func_wrap(
        "env",
        "game_gpu_render_pipeline_create_glass",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _pipeline_layout: u32, _shader: u32, _vertex_stride: u32,
         _instance_stride: u32, _color_format: u32, _depth_format: u32,
         _label_ptr: u32, _label_len: u32|
         -> Result<u32> { Ok(1) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_render_glass",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _target: u32, _pipeline: u32, _bind_group: u32,
         _vertex_buf: u32, _instance_buf: u32,
         _vertex_count: u32, _instance_count: u32|
         -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_render_pipeline_create_ghost",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _pipeline_layout: u32, _shader: u32, _vertex_stride: u32,
         _instance_stride: u32, _color_format: u32, _depth_format: u32,
         _label_ptr: u32, _label_len: u32|
         -> Result<u32> { Ok(1) },
    )?;
    linker.func_wrap(
        "env",
        "game_gpu_render_ghost",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>,
         _target: u32, _pipeline: u32, _bind_group: u32,
         _vertex_buf: u32, _instance_buf: u32,
         _vertex_count: u32, _instance_count: u32|
         -> Result<u32> { Ok(0) },
    )?;

    // Persistence. No IndexedDB under wasmtime — every load returns 0
    // ("not found") so Rust falls back to its defaults deterministically,
    // and every save is a no-op.
    linker.func_wrap(
        "env",
        "game_position_load",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _out_ptr: i32| -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_position_save",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _bytes_ptr: i32, _bytes_len: u32|
         -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_music_state_load",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _out_ptr: i32| -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_music_state_save",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _bytes_ptr: i32, _bytes_len: u32|
         -> Result<()> { Ok(()) },
    )?;
    linker.func_wrap(
        "env",
        "game_sfx_state_load",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _out_ptr: i32| -> Result<u32> { Ok(0) },
    )?;
    linker.func_wrap(
        "env",
        "game_sfx_state_save",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _bytes_ptr: i32, _bytes_len: u32|
         -> Result<()> { Ok(()) },
    )?;

    // Audio volume — no AudioContext under wasmtime, so this is inert.
    linker.func_wrap(
        "env",
        "game_audio_set_volume",
        |_caller: Caller<'_, Arc<Mutex<HostState>>>, _handle: u32, _volume_x1000: u32|
         -> Result<()> { Ok(()) },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `env.*` crossing the game is allowed to declare
    /// (`game/imports.allow`) must be satisfiable by this linker. When
    /// the game adds a boundary crossing (glass/ghost render, persistence,
    /// audio volume, …) and nobody mirrors it here, game.wasm fails to
    /// instantiate under wasmtime with `unknown import: env::…`. In CI the
    /// run step pipes through `tee`, so that crash exits 0 and the job
    /// stays green while seer measures *nothing* — history and summary
    /// freeze silently. This test turns that drift into a red `cargo test`
    /// at the source, so the error lands in front of us instead of rotting
    /// as stale S3 data.
    #[test]
    fn linker_satisfies_every_allowed_import() {
        let allow = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../game/imports.allow"
        ));
        let engine = Engine::default();
        let state = Arc::new(Mutex::new(HostState::new()));
        let mut store: Store<Arc<Mutex<HostState>>> = Store::new(&engine, state);
        let mut linker: Linker<Arc<Mutex<HostState>>> = Linker::new(&engine);
        wire_imports(&mut linker).expect("wire_imports");

        let mut missing = Vec::new();
        for line in allow.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (module, name) = line
                .split_once('.')
                .unwrap_or_else(|| panic!("imports.allow entry is not module.name: {line:?}"));
            if linker.get(&mut store, module, name).is_none() {
                missing.push(line.to_string());
            }
        }
        assert!(
            missing.is_empty(),
            "seer-host linker is missing {} import(s) present in game/imports.allow — game.wasm \
             will fail to instantiate under wasmtime and seer will measure nothing:\n  {}",
            missing.len(),
            missing.join("\n  ")
        );
    }
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
