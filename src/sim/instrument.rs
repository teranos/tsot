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

use std::io::IsTerminal;
use std::sync::{Mutex, OnceLock};

/// ANSI coloring is enabled iff stderr is a terminal. Detected once
/// at first use and cached. When piped (e.g., into a file or another
/// process) we emit plain text so the bytes stay grep-able.
fn ansi_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::io::stderr().is_terminal())
}

fn paint(code: &str, s: impl std::fmt::Display) -> String {
    if ansi_enabled() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn paint_red(s: impl std::fmt::Display) -> String { paint("31", s) }
pub fn paint_green(s: impl std::fmt::Display) -> String { paint("32", s) }
pub fn paint_yellow(s: impl std::fmt::Display) -> String { paint("33", s) }
pub fn paint_blue(s: impl std::fmt::Display) -> String { paint("34", s) }
pub fn paint_magenta(s: impl std::fmt::Display) -> String { paint("35", s) }
pub fn paint_cyan(s: impl std::fmt::Display) -> String { paint("36", s) }
pub fn paint_dim(s: impl std::fmt::Display) -> String { paint("2", s) }
pub fn paint_bold(s: impl std::fmt::Display) -> String { paint("1", s) }
pub fn paint_bold_green(s: impl std::fmt::Display) -> String { paint("1;32", s) }
pub fn paint_bold_yellow(s: impl std::fmt::Display) -> String { paint("1;33", s) }
pub fn paint_bold_red(s: impl std::fmt::Display) -> String { paint("1;31", s) }

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

/// Append `msg` to `log`. No stderr emit — the caller decides
/// whether and when to surface the buffered trace. Curve-sample
/// keeps the log per-game and dumps it only when the game had
/// failures, so clean games stay quiet (one END line) while erroring
/// games print full action context.
///
/// Live signals during a long game continue to come from elsewhere:
///   - watchdog heartbeat (curve-sample's 1s thread, prints current_op)
///   - engine HEARTBEAT (sim/run.rs:542, every 5s during outer loop)
///   - rollout-stall stderr dump (sim/step/mod.rs::run_to_end)
/// so the operator never loses sight of what the engine is doing
/// when something hangs.
pub fn tee_log(log: &mut Vec<String>, msg: String) {
    log.push(msg);
}
