//! roam's Sacred Error bus.
//!
//! Types live in the `sacred-error` crate at the repository root
//! (`crates/sacred-error/src/lib.rs`); TSOT consumes the same crate
//! via path dep so the wire shape is structurally guaranteed to
//! match between the two binaries — no parallel copies that drift.
//!
//! This module owns ONLY the roam-specific bits:
//!   - the thread-local buffer
//!   - the `"err-roam-N"` id namespace
//!   - the placeholder timestamp source (empty string until roam's
//!     Rust trace bus lands per docs/OBSERVABILITY.md Phase 2)
//!
//! Call sites use `crate::error::{push, drain, reset, next_id,
//! emit, emit_region, Error, Severity, Context, Anchor}` — the
//! same imports they used before the extraction.

pub use sacred_error::{Anchor, Context, Error, Severity};

use std::cell::RefCell;

/// Id prefix for monotonic counter. Distinct from TSOT's
/// `"err-rust"` so the JS-side dispatcher's id keying never collides
/// when both binaries push into the same renderer.
const ID_PREFIX: &str = "err-roam";

thread_local! {
    static ERRORS: RefCell<Vec<Error>> = const { RefCell::new(Vec::new()) };
    static ERROR_COUNTER: RefCell<u64> = const { RefCell::new(0) };
}

pub fn push(error: Error) {
    ERRORS.with(|c| c.borrow_mut().push(error));
}

pub fn drain() -> Vec<Error> {
    ERRORS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

pub fn reset() {
    let _ = drain();
    ERROR_COUNTER.with(|c| *c.borrow_mut() = 0);
}

pub fn next_id() -> String {
    ERROR_COUNTER.with(|c| {
        let mut n = c.borrow_mut();
        *n += 1;
        format!("{ID_PREFIX}-{}", *n)
    })
}

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
        // Placeholder until roam's Rust trace bus lands
        // (docs/OBSERVABILITY.md Phase 2). The Elm decoder accepts any
        // string here; empty is the sentinel for "no engine clock yet".
        at: String::new(),
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
        at: String::new(),
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
    // Tests here pin roam-specific behavior: the id-prefix is
    // "err-roam-", the buffer is per-thread, and emit/emit_region
    // round-trip through it correctly.

    use super::*;

    #[test]
    fn emit_pushes_to_buffer_and_returns_the_error() {
        reset();
        let e = emit(Severity::Warn, "test-surface", "test title", "test why");
        let drained = drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0], e);
        assert_eq!(drained[0].context.surface, "test-surface");
        assert_eq!(drained[0].severity, Severity::Warn);
        assert_eq!(drain().len(), 0);
    }

    #[test]
    fn next_id_is_monotonic_within_a_session_and_uses_roam_prefix() {
        reset();
        assert_eq!(next_id(), "err-roam-1");
        assert_eq!(next_id(), "err-roam-2");
        assert_eq!(next_id(), "err-roam-3");
        reset();
        assert_eq!(next_id(), "err-roam-1", "reset restarts the counter");
    }

    #[test]
    fn emit_region_sets_the_region_field() {
        reset();
        let e = emit_region(
            Severity::Error,
            "wasm-ffi",
            "roam_tick",
            "decode failed",
            "input bits invalid",
        );
        assert_eq!(e.context.region.as_deref(), Some("roam_tick"));
        let _ = drain();
    }
}
