// Host-side world state accumulated during a run. Every wasm→host
// boundary crossing mutates this via one of the linker.func_wrap
// bodies in imports.rs. Read at end of run by summary + report.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Metric {
    pub frame: u32,
    pub heap_bytes: u32,
    pub gpu_live: u32,
    pub gpu_bytes: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HotspotRecord {
    pub size: u32,
    pub align: u32,
    pub backtrace: String,
}

#[derive(Clone, Serialize, Deserialize)]
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

/// Per-commit structured artifact for the frontend to render.
/// Everything the diagnostic wants to show that isn't already in
/// summary.json / verdict.json / metrics.json / history.json lives
/// here. Written to SEER_COMMIT_REPORT_OUT_PATH at end of run.
#[derive(Serialize, Deserialize)]
pub struct CommitReport {
    pub hotspot_records: BTreeMap<u32, HotspotRecord>,
    pub gpu_records: BTreeMap<u32, GpuRecord>,
    pub errors_captured: Vec<String>,
    /// Total ledger entries recorded (before signal/noise filtering).
    /// Frontend shows "log_tail.len() of ledger_total".
    pub ledger_total: usize,
    /// Filtered + tailed subset of the host ledger. Noise (per-frame
    /// metric dumps, per-alloc hotspot detail, gpu inventory chatter)
    /// is dropped; each remaining line has the `seer_emit len=N: `
    /// wrapper stripped so the reader sees the actual emit content.
    pub log_tail: Vec<String>,
}

/// Max lines retained in CommitReport.log_tail. Older signal lines
/// are dropped once the tail passes this length.
pub const LOG_TAIL_MAX: usize = 50;

impl CommitReport {
    pub fn from_state(st: &HostState) -> Self {
        Self {
            hotspot_records: st.hotspot_records.clone(),
            gpu_records: st.gpu_records.clone(),
            errors_captured: st.errors_captured.clone(),
            ledger_total: st.ledger.len(),
            log_tail: filter_and_tail_ledger(&st.ledger, LOG_TAIL_MAX),
        }
    }
}

/// Filter noise + strip the `seer_emit len=N: ` wrapper + tail to
/// `max` lines. Pure fn; testable without a HostState.
pub fn filter_and_tail_ledger(ledger: &[String], max: usize) -> Vec<String> {
    const NOISY_PREFIXES: &[&str] = &[
        "[obs.metric",
        "[obs.hotspots",
        "[obs.hotspot]",
        "[obs.gpu.live]",
        "[obs.gpu.inventory]",
        "[live-buf",
        "[gpu-alloc",
    ];
    let filtered: Vec<String> = ledger
        .iter()
        .map(|l| unwrap_emit(l))
        .filter(|l| !NOISY_PREFIXES.iter().any(|p| l.starts_with(p)))
        .map(|l| l.to_string())
        .collect();
    let n = filtered.len();
    if n <= max {
        filtered
    } else {
        filtered[n - max..].to_vec()
    }
}

fn unwrap_emit(line: &str) -> &str {
    line.strip_prefix("seer_emit len=")
        .and_then(|s| s.split_once(": "))
        .map(|(_, rest)| rest)
        .unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_tail_drops_noisy_prefixes_and_strips_emit_wrapper() {
        let ledger = vec![
            String::from("seer_emit len=42: [seer.setup] booted"),
            String::from("seer_emit len=30: [obs.metric] frame=1"),
            String::from("seer_emit len=44: [seer.error id=1] boom"),
            String::from("seer_emit len=30: [obs.hotspots] cap=128"),
            String::from("seer_record_hotspot seq=7 size=131072 align=8 frames=6"),
        ];
        let tail = filter_and_tail_ledger(&ledger, 50);
        assert_eq!(
            tail,
            vec![
                "[seer.setup] booted".to_string(),
                "[seer.error id=1] boom".to_string(),
                "seer_record_hotspot seq=7 size=131072 align=8 frames=6".to_string(),
            ]
        );
    }

    #[test]
    fn log_tail_caps_at_max() {
        let ledger: Vec<String> = (0..100)
            .map(|i| format!("seer_emit len=10: [seer.tick] i={i}"))
            .collect();
        let tail = filter_and_tail_ledger(&ledger, 5);
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0], "[seer.tick] i=95");
        assert_eq!(tail[4], "[seer.tick] i=99");
    }

    #[test]
    fn commit_report_from_state_serializes_expected_shape() {
        let mut st = HostState::new();
        st.ledger
            .push("seer_emit len=20: [seer.boot] up".to_string());
        st.ledger
            .push("seer_emit len=30: [obs.metric] frame=1".to_string());
        st.errors_captured
            .push("[seer.error id=1] test".to_string());
        st.hotspot_records.insert(
            1,
            HotspotRecord {
                size: 65536,
                align: 8,
                backtrace: "frame0\nframe1".to_string(),
            },
        );
        st.gpu_records.insert(
            42,
            GpuRecord {
                kind: 1,
                size: 4096,
                backtrace: "gpu-frame".to_string(),
            },
        );

        let report = CommitReport::from_state(&st);
        assert_eq!(report.ledger_total, 2);
        assert_eq!(report.log_tail, vec!["[seer.boot] up".to_string()]);
        assert_eq!(report.errors_captured.len(), 1);
        assert_eq!(report.hotspot_records.get(&1).unwrap().size, 65536);
        assert_eq!(report.gpu_records.get(&42).unwrap().kind, 1);

        let json = serde_json::to_string(&report).unwrap();
        let round: CommitReport = serde_json::from_str(&json).unwrap();
        assert_eq!(round.ledger_total, 2);
        assert_eq!(round.log_tail.len(), 1);
        assert_eq!(round.hotspot_records.len(), 1);
        assert_eq!(round.gpu_records.len(), 1);
    }
}
