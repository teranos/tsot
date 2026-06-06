//! Cross-thread instrumentation point for "what is the engine doing
//! right now". The engine writes to `CURRENT_OP` at known sites
//! (turn-loop top, phase advance, AI pick, handler dispatch). A
//! watchdog in any consumer (curve-sample, evolve, etc.) reads it
//! from another thread and prints it as part of its heartbeat. When
//! a game hangs, the last-written string identifies the inner call
//! the engine is stuck inside.
//!
//! Cost: one `Mutex<String>` lock per instrumented site (sub-µs on a
//! single-thread contention pattern). Not on a hot loop body —
//! instrumentation sits at the boundaries of multi-millisecond inner
//! calls, so overhead is negligible.

use std::sync::{LazyLock, Mutex};

/// The current inner operation the engine is executing. Updated by
/// the engine at key boundaries; read by watchdogs.
pub static CURRENT_OP: Mutex<String> = Mutex::new(String::new());

/// Record the current operation. Callers stamp this immediately
/// before entering a multi-ms inner call so a hang inside that call
/// leaves a readable explanation.
pub fn set_current_op(s: impl Into<String>) {
    if let Ok(mut g) = CURRENT_OP.lock() {
        *g = s.into();
    }
}

/// Read the current operation. For watchdogs running on another
/// thread.
pub fn current_op() -> String {
    CURRENT_OP.lock().ok().map(|g| g.clone()).unwrap_or_default()
}

/// Append `msg` to `log` AND mirror it to stderr — with per-line
/// milestone dedupe. The internal `log` vector keeps every entry (no
/// data loss in the source). Stderr emits each unique line on these
/// occurrence counts: 1, 2, 5, 10, 50, 100, 500, 1000, ... — so a
/// tight pick/resolve cycle that fires the same line 100,000 times
/// shows up as ~10 emissions instead of 100,000. The alternation
/// rhythm is preserved across distinct lines (each gets its own
/// counter).
///
/// Why: a tight loop emitting two alternating lines (A, B, A, B, …)
/// defeats consecutive dedupe — each line is "different from the
/// previous." Per-line counting keeps both alternation rhythm and
/// log readability.
struct DedupeState {
    counts: std::collections::HashMap<String, u64>,
}

static DEDUPE: LazyLock<Mutex<DedupeState>> = LazyLock::new(|| {
    Mutex::new(DedupeState {
        counts: std::collections::HashMap::new(),
    })
});

const MILESTONES: &[u64] = &[
    1, 2, 5, 10, 50, 100, 500, 1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000,
];

pub fn tee_log(log: &mut Vec<String>, msg: String) {
    log.push(msg.clone());
    let Ok(mut state) = DEDUPE.lock() else { return };
    let count = state.counts.entry(msg.clone()).or_insert(0);
    *count += 1;
    let n = *count;
    if MILESTONES.contains(&n) {
        if n == 1 {
            eprintln!("[engine] {msg}");
        } else {
            eprintln!("[engine] {msg}  ⟨#{n}⟩");
        }
    }
}

/// Reset the per-line counters. Call between distinct units of work
/// (e.g., between games in curve-sample) so each game's milestones
/// are independent.
pub fn dedupe_reset() {
    if let Ok(mut state) = DEDUPE.lock() {
        state.counts.clear();
    }
}
