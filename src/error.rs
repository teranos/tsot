//! TSOT's Sacred Error bus.
//!
//! Types live in the `sacred-error` crate at the repository root
//! (`crates/sacred-error/src/lib.rs`); roam consumes the same crate
//! via path dep so the wire shape is structurally guaranteed to
//! match between the two binaries — no parallel copies that drift.
//!
//! This module owns ONLY the TSOT-specific bits:
//!   - the thread-local buffer
//!   - the `"err-rust-N"` id namespace
//!   - the `crate::trace::now_us()` timestamp source
//!
//! Call sites use `crate::error::{push, drain, reset, next_id,
//! emit, emit_region, Error, Severity, Context, Anchor}` — the
//! same imports they used before the extraction.

pub use sacred_error::{Anchor, Context, Error, Severity};

use std::cell::RefCell;

/// Id prefix for monotonic counter. Distinct from roam's
/// `"err-roam"` so the JS-side dispatcher's id keying never collides
/// when both binaries push into the same renderer (which today never
/// happens — they're separate pages — but the namespace separation
/// costs nothing and forbids the collision in advance).
const ID_PREFIX: &str = "err-rust";

thread_local! {
    /// Per-thread Error buffer. Drained by the wasm FFI into the
    /// envelope on every yield. Mirrors `crate::trace::TRACE`.
    static ERRORS: RefCell<Vec<Error>> = const { RefCell::new(Vec::new()) };
    /// Monotonic id source. Reset on each session via `reset()`.
    static ERROR_COUNTER: RefCell<u64> = const { RefCell::new(0) };
}

/// Push one Error onto the buffer. Always pushes (no enable gate
/// — errors are sacred per the axiom; suppression at this layer
/// would itself be a sanctity violation).
pub fn push(error: Error) {
    ERRORS.with(|c| c.borrow_mut().push(error));
}

/// Take the buffer's contents, leaving it empty. The wasm FFI
/// calls this after each call to attach to the envelope.
pub fn drain() -> Vec<Error> {
    ERRORS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Reset the buffer + id counter. Called at FFI boundaries before
/// the call's work begins so the envelope carries only THIS call's
/// errors (mirrors the pattern in `crate::trace::drain` + reset).
pub fn reset() {
    let _ = drain();
    ERROR_COUNTER.with(|c| *c.borrow_mut() = 0);
}

/// Mint a new monotonic Error id, namespaced so duplicates with
/// JS-side / Elm-side counters are impossible.
pub fn next_id() -> String {
    ERROR_COUNTER.with(|c| {
        let mut n = c.borrow_mut();
        *n += 1;
        format!("{ID_PREFIX}-{}", *n)
    })
}

/// Construct + push + return a new Error in one call. `surface` is
/// the contextual tag the JS-side router uses to pick the rendering
/// container. `title` is the one-line summary; `why` is the cause
/// chain (free-form, but include path + values so the developer can
/// act on it without opening a debugger).
pub fn emit(
    severity: Severity,
    surface: impl Into<String>,
    title: impl Into<String>,
    why: impl Into<String>,
) -> Error {
    let error = Error {
        id: next_id(),
        severity,
        context: Context {
            surface: surface.into(),
            region: None,
            anchor: None,
        },
        title: title.into(),
        why: why.into(),
        trace: vec![],
        raw: None,
        at: format!("{}us", crate::trace::now_us()),
        source: None,
        ffi_call: None,
        location: None,
        js_stack: None,
        raw_stderr: None,
        requires_reload: false,
    };
    push(error.clone());
    error
}

/// Variant of [`emit`] that also attaches a region tag for inline
/// surfaces (e.g. `region = "preset-dropdown"`).
pub fn emit_region(
    severity: Severity,
    surface: impl Into<String>,
    region: impl Into<String>,
    title: impl Into<String>,
    why: impl Into<String>,
) -> Error {
    let error = Error {
        id: next_id(),
        severity,
        context: Context {
            surface: surface.into(),
            region: Some(region.into()),
            anchor: None,
        },
        title: title.into(),
        why: why.into(),
        trace: vec![],
        raw: None,
        at: format!("{}us", crate::trace::now_us()),
        source: None,
        ffi_call: None,
        location: None,
        js_stack: None,
        raw_stderr: None,
        requires_reload: false,
    };
    push(error.clone());
    error
}

#[cfg(test)]
mod tests {
    // Wire-shape tests live in the `sacred-error` crate; those are
    // the cross-layer contract.
    //
    // Tests here pin TSOT-specific behavior: the id-prefix is
    // "err-rust-", the buffer is per-thread, reset restarts the
    // counter, and emit_region carries the region through.

    use super::*;

    #[test]
    fn emit_pushes_to_buffer_and_returns_the_error() {
        reset();
        let e = emit(Severity::Warn, "test-surface", "test title", "test why");
        let drained = drain();
        assert_eq!(drained.len(), 1, "buffer should have one entry");
        assert_eq!(drained[0], e);
        assert_eq!(drained[0].context.surface, "test-surface");
        assert_eq!(drained[0].severity, Severity::Warn);
        assert_eq!(drain().len(), 0, "drain should leave buffer empty");
    }

    #[test]
    fn next_id_is_monotonic_within_a_session_and_uses_tsot_prefix() {
        reset();
        assert_eq!(next_id(), "err-rust-1");
        assert_eq!(next_id(), "err-rust-2");
        assert_eq!(next_id(), "err-rust-3");
        reset();
        assert_eq!(next_id(), "err-rust-1", "reset restarts the counter");
    }

    #[test]
    fn emit_region_sets_the_region_field() {
        reset();
        let e = emit_region(
            Severity::Error,
            "deckbuilder",
            "preset-dropdown",
            "decode failed",
            "preset[2].cards empty",
        );
        assert_eq!(e.context.region.as_deref(), Some("preset-dropdown"));
        let _ = drain();
    }
}
