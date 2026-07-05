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
use rustc_demangle::demangle;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use wasmtime::*;

#[derive(Serialize, Deserialize, Clone)]
struct RunSummary {
    sha: String,
    when_unix: u64,
    frames: u32,
    first_frame: u32,
    last_frame: u32,
    heap_start_mb: f64,
    heap_end_mb: f64,
    d_heap_mb: f64,
    gpu_live_start: u32,
    gpu_live_end: u32,
    d_gpu_live: i64,
    gpu_bytes_start_mb: f64,
    gpu_bytes_end_mb: f64,
    d_gpu_bytes_mb: f64,
    leak_enabled: bool,
    report_url: String,
}

const HISTORY_CAP: usize = 20;

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

#[derive(Clone, Copy)]
struct Metric {
    frame: u32,
    heap_bytes: u32,
    gpu_live: u32,
    gpu_bytes: u32,
}

struct HostState {
    ledger: Vec<String>,
    hotspot_backtraces: BTreeMap<u32, String>,
    gpu_backtraces: BTreeMap<u32, String>,
    metrics: Vec<Metric>,
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
        gpu_backtraces: BTreeMap::new(),
        metrics: Vec::new(),
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
    // Same host-ledger pattern as seer_record_hotspot, keyed by gpu id.
    // Partitions the ledger by event type so seq spaces don't collide.
    linker.func_wrap(
        "env",
        "seer_record_gpu_event",
        |caller: Caller<'_, Arc<Mutex<HostState>>>, id: u32, kind: u32, size: u32| -> Result<()> {
            let bt = WasmBacktrace::force_capture(&caller);
            let rendered = render_wasm_backtrace(&bt);
            let kind_name = match kind {
                1 => "buffer",
                2 => "texture",
                3 => "shader",
                _ => "?",
            };
            let state = caller.data().clone();
            if let Ok(mut st) = state.lock() {
                st.gpu_backtraces
                    .insert(id, format!("kind={kind_name} size={size}\n{rendered}"));
                st.ledger.push(format!(
                    "seer_record_gpu_event id={id} kind={kind_name} size={size} frames={}",
                    bt.frames().len()
                ));
            }
            Ok(())
        },
    )?;

    // Structured per-frame metric. Cheap: no backtrace capture,
    // just four numbers. Feeds the HTML time-series chart.
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


    linker.func_wrap(
        "env",
        "seer_record_hotspot",
        |caller: Caller<'_, Arc<Mutex<HostState>>>, seq: u32, size: u32, align: u32| -> Result<()> {
            let bt = WasmBacktrace::force_capture(&caller);
            let rendered = render_wasm_backtrace(&bt);
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

    println!(
        "[host.gpu-backtraces] {} distinct gpu id entries:",
        st.gpu_backtraces.len()
    );
    for (id, bt) in st.gpu_backtraces.iter() {
        println!("[host.gpu id={id}]");
        println!("{bt}");
    }

    // Build this run's summary + load prior history + append + cap.
    let leak_enabled = std::env::var("SEER_LEAK")
        .ok()
        .is_some_and(|v| v == "1" || v == "true");
    let sha = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_string());
    let short_sha = sha.chars().take(7).collect::<String>();
    let when_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let summary = if !st.metrics.is_empty() {
        let first = &st.metrics[0];
        let last = &st.metrics[st.metrics.len() - 1];
        let mb = |b: u32| b as f64 / 1_048_576.0;
        RunSummary {
            sha: short_sha.clone(),
            when_unix,
            frames: st.metrics.len() as u32,
            first_frame: first.frame,
            last_frame: last.frame,
            heap_start_mb: mb(first.heap_bytes),
            heap_end_mb: mb(last.heap_bytes),
            d_heap_mb: mb(last.heap_bytes) - mb(first.heap_bytes),
            gpu_live_start: first.gpu_live,
            gpu_live_end: last.gpu_live,
            d_gpu_live: last.gpu_live as i64 - first.gpu_live as i64,
            gpu_bytes_start_mb: mb(first.gpu_bytes),
            gpu_bytes_end_mb: mb(last.gpu_bytes),
            d_gpu_bytes_mb: mb(last.gpu_bytes) - mb(first.gpu_bytes),
            leak_enabled,
            report_url: format!("/perf/{sha}/report.html"),
        }
    } else {
        RunSummary {
            sha: short_sha.clone(),
            when_unix,
            frames: 0,
            first_frame: 0,
            last_frame: 0,
            heap_start_mb: 0.0,
            heap_end_mb: 0.0,
            d_heap_mb: 0.0,
            gpu_live_start: 0,
            gpu_live_end: 0,
            d_gpu_live: 0,
            gpu_bytes_start_mb: 0.0,
            gpu_bytes_end_mb: 0.0,
            d_gpu_bytes_mb: 0.0,
            leak_enabled,
            report_url: format!("/perf/{sha}/report.html"),
        }
    };

    let mut history: Vec<RunSummary> = if let Ok(path) = std::env::var("SEER_HISTORY_IN_PATH") {
        match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    history.push(summary.clone());
    if history.len() > HISTORY_CAP {
        let n = history.len();
        history.drain(0..n - HISTORY_CAP);
    }

    // Write history JSON (workflow uploads it back to S3).
    if let Ok(out_path) = std::env::var("SEER_HISTORY_OUT_PATH") {
        match serde_json::to_string_pretty(&history) {
            Ok(s) => match std::fs::write(&out_path, s) {
                Ok(_) => println!("[host] wrote history: {out_path} ({} entries)", history.len()),
                Err(e) => println!("[host] history write failed: {e}"),
            },
            Err(e) => println!("[host] history serialize failed: {e}"),
        }
    }

    // Write single-run summary JSON.
    if let Ok(out_path) = std::env::var("SEER_SUMMARY_OUT_PATH") {
        match serde_json::to_string_pretty(&summary) {
            Ok(s) => {
                let _ = std::fs::write(&out_path, s);
            }
            Err(_) => {}
        }
    }

    // Write the HTML report if SEER_REPORT_PATH is set, else default
    // to ./report.html next to the log.
    let report_path = std::env::var("SEER_REPORT_PATH").unwrap_or_else(|_| "report.html".to_string());
    match render_html_report(&st, &report_path, &history, &summary) {
        Ok(_) => println!("[host] wrote HTML report: {report_path} ({} metrics)", st.metrics.len()),
        Err(e) => println!("[host] HTML report write failed: {e}"),
    }

    Ok(())
}

fn render_html_report(st: &HostState, path: &str, history: &[RunSummary], current: &RunSummary) -> Result<()> {
    let m = &st.metrics;
    let n = m.len();

    // Chart geometry.
    let w: u32 = 1000;
    let h: u32 = 320;
    let pad_l: u32 = 60;
    let pad_r: u32 = 20;
    let pad_t: u32 = 20;
    let pad_b: u32 = 40;
    let plot_w = w - pad_l - pad_r;
    let plot_h = h - pad_t - pad_b;

    let (last_frame, max_heap, max_gpu_bytes, max_gpu_live) = if n > 0 {
        let last = m[n - 1].frame.max(1);
        let mh = m.iter().map(|x| x.heap_bytes).max().unwrap_or(1).max(1);
        let mgb = m.iter().map(|x| x.gpu_bytes).max().unwrap_or(1).max(1);
        let mgl = m.iter().map(|x| x.gpu_live).max().unwrap_or(1).max(1);
        (last, mh, mgb, mgl)
    } else {
        (1, 1, 1, 1)
    };

    let x_of = |frame: u32| -> f32 {
        pad_l as f32 + (frame as f32 / last_frame as f32) * plot_w as f32
    };
    let y_of_norm = |v: f32| -> f32 {
        pad_t as f32 + plot_h as f32 - v.clamp(0.0, 1.0) * plot_h as f32
    };

    let build_path = |vals: &[(u32, f32)]| -> String {
        if vals.is_empty() {
            return String::new();
        }
        let mut s = String::new();
        for (i, (frame, v)) in vals.iter().enumerate() {
            let x = x_of(*frame);
            let y = y_of_norm(*v);
            if i == 0 {
                s.push_str(&format!("M {x:.1} {y:.1}"));
            } else {
                s.push_str(&format!(" L {x:.1} {y:.1}"));
            }
        }
        s
    };

    let heap_points: Vec<(u32, f32)> = m
        .iter()
        .map(|x| (x.frame, x.heap_bytes as f32 / max_heap as f32))
        .collect();
    let gpu_bytes_points: Vec<(u32, f32)> = m
        .iter()
        .map(|x| (x.frame, x.gpu_bytes as f32 / max_gpu_bytes as f32))
        .collect();
    let gpu_live_points: Vec<(u32, f32)> = m
        .iter()
        .map(|x| (x.frame, x.gpu_live as f32 / max_gpu_live as f32))
        .collect();

    let heap_path = build_path(&heap_points);
    let gpu_bytes_path = build_path(&gpu_bytes_points);
    let gpu_live_path = build_path(&gpu_live_points);

    // Before/after snapshots: first and last metric.
    let before_after_html = if n >= 2 {
        let a = m[0];
        let b = m[n - 1];
        let dh = b.heap_bytes as i64 - a.heap_bytes as i64;
        let dgb = b.gpu_bytes as i64 - a.gpu_bytes as i64;
        let dgl = b.gpu_live as i64 - a.gpu_live as i64;
        format!(
            r#"<div class="grid2">
  <div class="card">
    <h3>BEFORE — frame {af}</h3>
    <div class="stat">heap: {ah:.2} MB</div>
    <div class="stat">gpu live: {agl}</div>
    <div class="stat">gpu bytes: {agb:.2} MB</div>
  </div>
  <div class="card">
    <h3>AFTER — frame {bf}</h3>
    <div class="stat">heap: {bh:.2} MB <span class="delta">({dh_s})</span></div>
    <div class="stat">gpu live: {bgl} <span class="delta">({dgl_s})</span></div>
    <div class="stat">gpu bytes: {bgb:.2} MB <span class="delta">({dgb_s})</span></div>
  </div>
</div>"#,
            af = a.frame,
            ah = a.heap_bytes as f64 / 1_048_576.0,
            agl = a.gpu_live,
            agb = a.gpu_bytes as f64 / 1_048_576.0,
            bf = b.frame,
            bh = b.heap_bytes as f64 / 1_048_576.0,
            dh_s = fmt_delta_bytes(dh),
            bgl = b.gpu_live,
            dgl_s = fmt_delta_count(dgl),
            bgb = b.gpu_bytes as f64 / 1_048_576.0,
            dgb_s = fmt_delta_bytes(dgb),
        )
    } else {
        String::from(r#"<div class="banner">Not enough metrics for before/after.</div>"#)
    };

    // Metric table.
    let mut table_rows = String::new();
    let stride = (n / 20).max(1);
    let mut prev: Option<Metric> = None;
    for (i, met) in m.iter().enumerate() {
        if i.is_multiple_of(stride) || i + 1 == n {
            let dh = prev.map(|p| met.heap_bytes as i64 - p.heap_bytes as i64).unwrap_or(0);
            let dgl = prev.map(|p| met.gpu_live as i64 - p.gpu_live as i64).unwrap_or(0);
            let dgb = prev.map(|p| met.gpu_bytes as i64 - p.gpu_bytes as i64).unwrap_or(0);
            table_rows.push_str(&format!(
                r#"<tr><td>{}</td><td>{:.2}</td><td class="{}">{}</td><td>{}</td><td class="{}">{}</td><td>{:.2}</td><td class="{}">{}</td></tr>"#,
                met.frame,
                met.heap_bytes as f64 / 1_048_576.0,
                delta_class(dh),
                fmt_delta_bytes(dh),
                met.gpu_live,
                delta_class(dgl),
                fmt_delta_count(dgl),
                met.gpu_bytes as f64 / 1_048_576.0,
                delta_class(dgb),
                fmt_delta_bytes(dgb),
            ));
            prev = Some(*met);
        }
    }

    // Backtraces block.
    let mut bt_html = String::new();
    for (id, bt) in st.gpu_backtraces.iter().take(15) {
        bt_html.push_str(&format!(
            r#"<details><summary>gpu id={id}</summary><pre>{}</pre></details>"#,
            html_escape(bt)
        ));
    }

    let sha = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_string());
    let build_ts = std::env::var("GITHUB_RUN_STARTED_AT")
        .unwrap_or_else(|_| chrono_like_now());

    // Recent runs table — newest-first, one row per prior run.
    let mut history_rows = String::new();
    let mut rev_hist: Vec<&RunSummary> = history.iter().collect();
    rev_hist.reverse();
    for h in rev_hist.iter().take(15) {
        let is_current = h.sha == current.sha && h.when_unix == current.when_unix;
        let mark = if is_current { "→" } else { " " };
        let leak_marker = if h.leak_enabled { " · leak=on" } else { "" };
        history_rows.push_str(&format!(
            r#"<tr class="{cls}"><td>{mark}</td><td><a href="{url}">{sha}</a></td><td>{when}</td><td class="{dhc}">{dh}</td><td class="{dgc}">{dgl}</td><td class="{dbc}">{dgb}</td><td class="dim">{leak}</td></tr>"#,
            cls = if is_current { "current" } else { "" },
            mark = mark,
            url = h.report_url,
            sha = h.sha,
            when = h.when_unix,
            dhc = delta_class(h.d_heap_mb as i64),
            dh = fmt_delta_mb(h.d_heap_mb),
            dgc = delta_class(h.d_gpu_live),
            dgl = fmt_delta_count(h.d_gpu_live),
            dbc = delta_class(h.d_gpu_bytes_mb as i64),
            dgb = fmt_delta_mb(h.d_gpu_bytes_mb),
            leak = leak_marker,
        ));
    }
    let history_html = if history.is_empty() {
        String::from(r#"<div class="banner">No prior history (first run).</div>"#)
    } else {
        format!(
            r#"<table>
    <tr><th></th><th>sha</th><th>when (unix)</th><th>Δheap</th><th>Δgpu live</th><th>Δgpu bytes</th><th>flags</th></tr>
    {history_rows}
</table>"#
        )
    };

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en"><head>
<meta charset="utf-8">
<title>seer diagnostic report</title>
<style>
  :root {{
    --bg: #0a0e14; --fg: #cbd5e1; --dim: #64748b; --line: #334155;
    --panel: #121821; --accent: #22d3ee; --accent2: #f472b6; --accent3: #eab308;
    --up: #f87171; --down: #22c55e; --flat: #64748b;
  }}
  body {{ margin: 0; background: var(--bg); color: var(--fg); font-family: ui-monospace, monospace; padding: 24px; max-width: 1200px; margin: auto; }}
  h1 {{ color: #f1f5f9; font-size: 22px; margin: 4px 0 16px 0; }}
  h2 {{ color: #f1f5f9; font-size: 15px; margin: 32px 0 12px 0; text-transform: uppercase; letter-spacing: 0.06em; }}
  h3 {{ color: var(--dim); font-size: 12px; margin: 0 0 8px 0; letter-spacing: 0.08em; text-transform: uppercase; }}
  .banner {{ background: var(--panel); padding: 12px 16px; border-left: 3px solid var(--accent); font-size: 13px; }}
  .grid2 {{ display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }}
  .card {{ background: var(--panel); padding: 16px; border-radius: 4px; }}
  .stat {{ font-size: 14px; margin: 4px 0; }}
  .delta {{ color: var(--dim); }}
  table {{ border-collapse: collapse; width: 100%; font-size: 12px; }}
  th, td {{ text-align: left; padding: 5px 10px; border-bottom: 1px solid var(--line); }}
  th {{ color: var(--dim); font-weight: normal; text-transform: uppercase; letter-spacing: 0.06em; font-size: 11px; }}
  td.up {{ color: var(--up); }}
  td.down {{ color: var(--down); }}
  td.flat {{ color: var(--flat); }}
  td.dim {{ color: var(--dim); }}
  tr.current {{ background: rgba(34, 211, 238, 0.08); }}
  tr.current td:first-child {{ color: var(--accent); font-weight: bold; }}
  a {{ color: var(--accent); text-decoration: none; }}
  a:hover {{ text-decoration: underline; }}
  svg {{ display: block; background: var(--panel); border-radius: 4px; margin: 8px 0; }}
  .legend {{ display: flex; gap: 20px; font-size: 12px; margin: 4px 0 12px 0; }}
  .swatch {{ display: inline-block; width: 12px; height: 3px; margin-right: 6px; vertical-align: middle; }}
  details {{ background: var(--panel); padding: 8px 12px; margin: 4px 0; border-radius: 3px; }}
  summary {{ cursor: pointer; font-size: 12px; color: var(--dim); }}
  pre {{ font-size: 11px; overflow-x: auto; color: var(--fg); margin: 8px 0 0 0; }}
  code {{ color: var(--accent); }}
</style>
</head><body>
  <h1>seer diagnostic report</h1>
  <div class="banner">
    build: <code>{sha}</code> · started: <code>{build_ts}</code> · metrics: <code>{n}</code> frames · last frame: <code>{last_frame}</code> · leak: <code>{leak_str}</code>
  </div>

  <h2>Recent runs</h2>
  {history_html}

  <h2>Before / After</h2>
  {before_after_html}

  <h2>Time series (each series normalised to its own max)</h2>
  <div class="legend">
    <span><span class="swatch" style="background:#22d3ee"></span>heap bytes (max {mh:.2} MB)</span>
    <span><span class="swatch" style="background:#f472b6"></span>gpu bytes (max {mgb:.2} MB)</span>
    <span><span class="swatch" style="background:#eab308"></span>gpu live count (max {mgl})</span>
  </div>
  <svg width="{w}" height="{h}" viewBox="0 0 {w} {h}">
    <line x1="{pad_l}" y1="{pad_t}" x2="{pad_l}" y2="{plot_bottom}" stroke="#64748b" stroke-width="1"/>
    <line x1="{pad_l}" y1="{plot_bottom}" x2="{plot_right}" y2="{plot_bottom}" stroke="#64748b" stroke-width="1"/>
    <path d="{heap_path}" stroke="#22d3ee" fill="none" stroke-width="1.5"/>
    <path d="{gpu_bytes_path}" stroke="#f472b6" fill="none" stroke-width="1.5"/>
    <path d="{gpu_live_path}" stroke="#eab308" fill="none" stroke-width="1.5"/>
    <text x="{pad_l}" y="{text_below}" fill="#64748b" font-size="11" font-family="ui-monospace,monospace">frame 0</text>
    <text x="{text_right_x}" y="{text_below}" text-anchor="end" fill="#64748b" font-size="11" font-family="ui-monospace,monospace">frame {last_frame}</text>
  </svg>

  <h2>Metric table (sampled)</h2>
  <table>
    <tr><th>frame</th><th>heap MB</th><th>Δheap</th><th>gpu live</th><th>Δlive</th><th>gpu MB</th><th>Δgpu MB</th></tr>
    {table_rows}
  </table>

  <h2>GPU allocation call stacks (from wasmtime WasmBacktrace)</h2>
  {bt_html}
</body></html>
"##,
        sha = sha,
        build_ts = build_ts,
        n = n,
        leak_str = if current.leak_enabled { "ON" } else { "off" },
        history_html = history_html,
        last_frame = last_frame,
        before_after_html = before_after_html,
        w = w,
        h = h,
        pad_l = pad_l,
        pad_t = pad_t,
        plot_bottom = pad_t + plot_h,
        plot_right = pad_l + plot_w,
        text_below = pad_t + plot_h + 20,
        text_right_x = pad_l + plot_w,
        heap_path = heap_path,
        gpu_bytes_path = gpu_bytes_path,
        gpu_live_path = gpu_live_path,
        mh = max_heap as f64 / 1_048_576.0,
        mgb = max_gpu_bytes as f64 / 1_048_576.0,
        mgl = max_gpu_live,
        table_rows = table_rows,
        bt_html = bt_html,
    );

    std::fs::write(path, html)?;
    Ok(())
}

fn fmt_delta_bytes(d: i64) -> String {
    if d == 0 {
        "flat".to_string()
    } else {
        let mb = d.abs() as f64 / 1_048_576.0;
        if d > 0 {
            format!("+{mb:.2} MB")
        } else {
            format!("-{mb:.2} MB")
        }
    }
}

fn fmt_delta_mb(d: f64) -> String {
    if d.abs() < 0.005 {
        "flat".to_string()
    } else if d > 0.0 {
        format!("+{d:.2} MB")
    } else {
        format!("{d:.2} MB")
    }
}

fn fmt_delta_count(d: i64) -> String {
    if d == 0 {
        "flat".to_string()
    } else if d > 0 {
        format!("+{d}")
    } else {
        format!("{d}")
    }
}

fn delta_class(d: i64) -> &'static str {
    if d > 0 {
        "up"
    } else if d < 0 {
        "down"
    } else {
        "flat"
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn chrono_like_now() -> String {
    // No chrono dep — timestamp from system_time in unix seconds is fine.
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("unix={}", d.as_secs()),
        Err(_) => "unknown".to_string(),
    }
}
