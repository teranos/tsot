// RunSummary: what one seer-host invocation produced, serialized to
// summary.json + appended to history.json. Verdict: the pass/fail
// decision against configurable thresholds — this is the leak-catcher
// that turns the report from "here's data" into "CI failed at line X".

use serde::{Deserialize, Serialize};

use crate::state::HostState;

#[derive(Serialize, Deserialize, Clone)]
pub struct RunSummary {
    pub sha: String,
    pub when_unix: u64,
    pub frames: u32,
    pub first_frame: u32,
    pub last_frame: u32,
    pub heap_start_mb: f64,
    pub heap_end_mb: f64,
    pub d_heap_mb: f64,
    pub gpu_live_start: u32,
    pub gpu_live_end: u32,
    pub d_gpu_live: i64,
    pub gpu_bytes_start_mb: f64,
    pub gpu_bytes_end_mb: f64,
    pub d_gpu_bytes_mb: f64,
    pub leak_enabled: bool,
    pub report_url: String,
    // Populated only when running in GitHub Actions; empty otherwise.
    // #[serde(default)] keeps deserialization working against older
    // history.json entries that predate this field.
    #[serde(default)]
    pub ci_run_url: String,
    #[serde(default = "default_true")]
    pub verdict_passed: bool,
    #[serde(default)]
    pub verdict_violations: Vec<String>,
    /// Wall-clock seconds seer-host spent running (from main() entry
    /// to end). Approximates CI job duration for the seer-host step.
    #[serde(default)]
    pub duration_secs: u64,
    /// Size of the seer.wasm module in bytes, measured at the boundary
    /// where seer-host loads it. Regression signal for bloat: every
    /// commit records its wasm size; the recent-runs table shows the
    /// delta so wasm-bindgen creep, added deps, or lost optimisation
    /// becomes obvious in one column.
    #[serde(default)]
    pub wasm_bytes: u64,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
pub struct Verdict {
    pub passed: bool,
    pub violations: Vec<String>,
    pub thresholds: VerdictThresholds,
}

#[derive(Serialize, Clone, Copy)]
pub struct VerdictThresholds {
    pub max_d_heap_mb: f64,
    pub max_d_gpu_live: i64,
    pub max_d_gpu_bytes_mb: f64,
}

pub fn compute_verdict(summary: &RunSummary) -> Verdict {
    let t = VerdictThresholds {
        max_d_heap_mb: env_f64("SEER_MAX_D_HEAP_MB", 1.0),
        max_d_gpu_live: env_i64("SEER_MAX_D_GPU_LIVE", 5),
        max_d_gpu_bytes_mb: env_f64("SEER_MAX_D_GPU_BYTES_MB", 0.5),
    };
    let mut violations: Vec<String> = Vec::new();
    if summary.d_heap_mb > t.max_d_heap_mb {
        violations.push(format!(
            "Δheap = {:.3} MB > threshold {:.3} MB",
            summary.d_heap_mb, t.max_d_heap_mb
        ));
    }
    if summary.d_gpu_live > t.max_d_gpu_live {
        violations.push(format!(
            "Δgpu_live = +{} > threshold {}",
            summary.d_gpu_live, t.max_d_gpu_live
        ));
    }
    if summary.d_gpu_bytes_mb > t.max_d_gpu_bytes_mb {
        violations.push(format!(
            "Δgpu_bytes = {:.3} MB > threshold {:.3} MB",
            summary.d_gpu_bytes_mb, t.max_d_gpu_bytes_mb
        ));
    }
    Verdict {
        passed: violations.is_empty(),
        violations,
        thresholds: t,
    }
}

pub fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

pub fn env_i64(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Truncate a full 40-char SHA to the conventional 7-char display
/// form. Safe against already-short input (returns as-is if shorter).
pub fn short_sha(sha: &str) -> &str {
    let end = sha.len().min(7);
    &sha[..end]
}

/// Build the RunSummary from the host state at end of run.
/// Verdict fields are left at their defaults; caller runs
/// compute_verdict and patches them in.
///
/// `sha` is the full 40-char sha and is what gets stored in the
/// summary + used in URLs. Display code calls `short_sha(&summary.sha)`
/// when it wants the 7-char form.
pub fn build_summary(st: &HostState, sha: &str, ci_run_url: &str) -> RunSummary {
    let leak_enabled = std::env::var("SEER_LEAK")
        .ok()
        .is_some_and(|v| v == "1" || v == "true");
    let when_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (
        frames,
        first_frame,
        last_frame,
        heap_start_mb,
        heap_end_mb,
        gpu_live_start,
        gpu_live_end,
        gpu_bytes_start_mb,
        gpu_bytes_end_mb,
    ) = if !st.metrics.is_empty() {
        let first = &st.metrics[0];
        let last = &st.metrics[st.metrics.len() - 1];
        let mb = |b: u32| b as f64 / 1_048_576.0;
        (
            st.metrics.len() as u32,
            first.frame,
            last.frame,
            mb(first.heap_bytes),
            mb(last.heap_bytes),
            first.gpu_live,
            last.gpu_live,
            mb(first.gpu_bytes),
            mb(last.gpu_bytes),
        )
    } else {
        (0, 0, 0, 0.0, 0.0, 0, 0, 0.0, 0.0)
    };
    RunSummary {
        sha: sha.to_string(),
        when_unix,
        frames,
        first_frame,
        last_frame,
        heap_start_mb,
        heap_end_mb,
        d_heap_mb: heap_end_mb - heap_start_mb,
        gpu_live_start,
        gpu_live_end,
        d_gpu_live: gpu_live_end as i64 - gpu_live_start as i64,
        gpu_bytes_start_mb,
        gpu_bytes_end_mb,
        d_gpu_bytes_mb: gpu_bytes_end_mb - gpu_bytes_start_mb,
        leak_enabled,
        report_url: format!("/perf/{sha}/report.html"),
        ci_run_url: ci_run_url.to_string(),
        verdict_passed: true,
        verdict_violations: Vec::new(),
        duration_secs: 0,
        wasm_bytes: 0,
    }
}
