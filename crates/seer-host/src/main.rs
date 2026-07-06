mod imports;
mod state;
mod summary;

use anyhow::{Context, Result, anyhow};
use std::sync::{Arc, Mutex};
use wasmtime::*;

use crate::imports::wire_imports;
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

    // Best-effort publish to seer-summary/v1. Failure is logged and
    // swallowed; the diagnostic never blocks on the network.
    if let Ok(bootstrap) = std::env::var("SEER_P2P_BOOTSTRAP") {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build tokio runtime for p2p publish")?;
        let deadline = std::time::Duration::from_secs(6);
        match rt.block_on(seer_p2p::publish_summary(&summary, &bootstrap, deadline)) {
            Ok(_) => println!("[host.p2p] summary published to {}", seer_p2p::SEER_SUMMARY_TOPIC),
            Err(e) => println!("[host.p2p] publish skipped: {e}"),
        }
    }

    Ok(())
}
