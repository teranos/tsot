// Wasmtime host binary for the seer wasm module.
//
// Founding principle: every wasm→host boundary crossing is a Rust host
// function you own. The four imports the wasm module expects are wired
// in imports.rs; the state they mutate lives in state.rs; the
// interpretation (summary + verdict) lives in summary.rs; the HTML
// output lives in report.rs. main.rs just runs the ceremony.

mod imports;
mod report;
mod state;
mod summary;

use anyhow::{Context, Result, anyhow};
use std::sync::{Arc, Mutex};
use wasmtime::*;

use crate::imports::wire_imports;
use crate::report::render_html_report;
use crate::state::{CommitReport, HostState};
use crate::summary::{build_summary, compute_verdict};

const HISTORY_CAP: usize = 20;

fn main() -> Result<()> {
    let host_start = std::time::Instant::now();
    let wasm_path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: seer-host <path-to-seer.wasm>"))?;

    println!("[host] engine init");
    let engine = Engine::default();
    println!("[host] loading module: {wasm_path}");
    let module = Module::from_file(&engine, &wasm_path)
        .with_context(|| format!("loading module from {wasm_path}"))?;

    let state = Arc::new(Mutex::new(HostState::new()));
    let mut store: Store<Arc<Mutex<HostState>>> = Store::new(&engine, state.clone());
    let mut linker: Linker<Arc<Mutex<HostState>>> = Linker::new(&engine);
    wire_imports(&mut linker)?;

    println!("[host] instantiating");
    let instance = linker.instantiate(&mut store, &module)?;

    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .context("seer.wasm must export a `run` function")?;

    println!("[host] calling run()");
    run.call(&mut store, ())?;
    println!("[host] run() returned");

    let st = state
        .lock()
        .map_err(|e| anyhow!("state mutex poisoned: {e}"))?;

    println!(
        "[host.ledger] {} host-function calls recorded during run():",
        st.ledger.len()
    );
    for entry in st.ledger.iter() {
        println!("  {entry}");
    }

    println!(
        "[host.hotspot-records] {} distinct seq entries:",
        st.hotspot_records.len()
    );
    for (seq, r) in st.hotspot_records.iter() {
        println!(
            "[host.hotspot seq={seq}] size={} align={}",
            r.size, r.align
        );
        println!("{}", r.backtrace);
    }

    println!(
        "[host.gpu-records] {} distinct gpu id entries:",
        st.gpu_records.len()
    );
    for (id, r) in st.gpu_records.iter() {
        println!(
            "[host.gpu id={id}] kind={} size={}",
            state::kind_name(r.kind),
            r.size
        );
        println!("{}", r.backtrace);
    }

    let sha = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_string());
    let ci_run_url = match (
        std::env::var("GITHUB_REPOSITORY"),
        std::env::var("GITHUB_RUN_ID"),
    ) {
        (Ok(repo), Ok(run_id)) => format!("https://github.com/{repo}/actions/runs/{run_id}"),
        _ => String::new(),
    };

    let mut summary = build_summary(&st, &sha, &ci_run_url);
    summary.duration_secs = host_start.elapsed().as_secs();
    // Measure the wasm module's size at the same path the runtime
    // loaded — captures whatever the workflow just built + uploaded.
    summary.wasm_bytes = std::fs::metadata(&wasm_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let verdict = compute_verdict(&summary);
    summary.verdict_passed = verdict.passed;
    summary.verdict_violations = verdict.violations.clone();

    // Verdict JSON — workflow reads this after uploading and sets its
    // own exit code accordingly. seer-host itself always exits 0 so
    // the failing report still uploads (that's the whole point).
    if let Ok(out_path) = std::env::var("SEER_VERDICT_OUT_PATH")
        && let Ok(s) = serde_json::to_string_pretty(&verdict)
    {
        let _ = std::fs::write(&out_path, s);
    }

    if verdict.passed {
        println!("[host.verdict] PASS");
    } else {
        println!(
            "[host.verdict] FAIL ({} violations):",
            verdict.violations.len()
        );
        for v in &verdict.violations {
            println!("  - {v}");
        }
    }

    let mut history: Vec<summary::RunSummary> = if let Ok(path) =
        std::env::var("SEER_HISTORY_IN_PATH")
    {
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

    if let Ok(out_path) = std::env::var("SEER_HISTORY_OUT_PATH") {
        match serde_json::to_string_pretty(&history) {
            Ok(s) => match std::fs::write(&out_path, s) {
                Ok(_) => println!(
                    "[host] wrote history: {out_path} ({} entries)",
                    history.len()
                ),
                Err(e) => println!("[host] history write failed: {e}"),
            },
            Err(e) => println!("[host] history serialize failed: {e}"),
        }
    }

    if let Ok(out_path) = std::env::var("SEER_SUMMARY_OUT_PATH")
        && let Ok(s) = serde_json::to_string_pretty(&summary)
    {
        let _ = std::fs::write(&out_path, s);
    }

    // Write metrics.json for THIS run — the workflow uploads it to
    // /perf/<sha>/metrics.json so future runs' reports can embed
    // sparklines for each historical commit in the gallery.
    if let Ok(out_path) = std::env::var("SEER_METRICS_OUT_PATH")
        && let Ok(s) = serde_json::to_string(&st.metrics)
    {
        let _ = std::fs::write(&out_path, s);
    }

    // Write report.json — the per-commit structured artifact the
    // frontend renders. Bundles hotspot records, gpu records, sacred
    // errors, and the filtered log tail from this run. Emitted only
    // when the env var is set (CI does; local dev opts in).
    if let Ok(out_path) = std::env::var("SEER_COMMIT_REPORT_OUT_PATH") {
        let report = CommitReport::from_state(&st);
        match serde_json::to_string_pretty(&report) {
            Ok(s) => match std::fs::write(&out_path, s) {
                Ok(_) => println!(
                    "[host] wrote commit report: {out_path} ({} hotspots, {} gpu, {} errors, log_tail={}/{})",
                    report.hotspot_records.len(),
                    report.gpu_records.len(),
                    report.errors_captured.len(),
                    report.log_tail.len(),
                    report.ledger_total,
                ),
                Err(e) => println!("[host] report write failed: {e}"),
            },
            Err(e) => println!("[host] report serialize failed: {e}"),
        }
    }

    // Load prior commits' metrics from SEER_METRICS_DIR (populated by
    // the workflow via `aws s3 cp` of /perf/<sha>/metrics.json files
    // for the last N entries in history). Missing entries render as
    // no-sparkline placeholders — graceful across first-run and
    // history-truncation scenarios.
    let prior_metrics = load_prior_metrics();

    let report_path =
        std::env::var("SEER_REPORT_PATH").unwrap_or_else(|_| "report.html".to_string());
    match render_html_report(&st, &report_path, &history, &summary, &prior_metrics) {
        Ok(_) => println!(
            "[host] wrote HTML report: {report_path} ({} metrics)",
            st.metrics.len()
        ),
        Err(e) => println!("[host] HTML report write failed: {e}"),
    }

    Ok(())
}

/// Read prior commits' metrics from `SEER_METRICS_DIR/<sha>.json`.
/// Empty map if the env var is unset or the directory is missing.
/// Each entry: full-sha → Vec<Metric> for that commit's run. Keyed
/// by whatever the workflow named the file — currently the same
/// full sha the workflow uses for S3 upload paths, which matches
/// `RunSummary.sha`.
fn load_prior_metrics() -> std::collections::BTreeMap<String, Vec<state::Metric>> {
    let mut map: std::collections::BTreeMap<String, Vec<state::Metric>> =
        std::collections::BTreeMap::new();
    let Ok(dir) = std::env::var("SEER_METRICS_DIR") else {
        return map;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(metrics) = serde_json::from_str::<Vec<state::Metric>>(&text) {
            map.insert(stem.to_string(), metrics);
        }
    }
    println!("[host] loaded prior metrics for {} shas from {dir}", map.len());
    map
}
