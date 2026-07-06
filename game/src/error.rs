// Seer's Sacred Error bus. Ported from rave/src/error.rs.
//
// Types live in `sacred-error` (crates/sacred-error/). This module
// owns only the seer-specific bits:
//   - thread-local ERRORS ring
//   - "err-seer-N" id namespace
//   - `emit_region` helper matching rave's shape
//
// Axiom (from repo CLAUDE.md): errors are sacred — first-class
// citizens, never collapsed, dropped, swallowed. Seer's obs bus
// drains any captured errors at end of run so they surface in the
// diagnostic report alongside the memory + GPU signals.

pub use sacred_error::{Anchor, Context, Error, Severity};

use std::cell::RefCell;

const ID_PREFIX: &str = "err-seer";

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

pub fn count() -> usize {
    ERRORS.with(|c| c.borrow().len())
}

pub fn next_id() -> String {
    ERROR_COUNTER.with(|c| {
        let mut n = c.borrow_mut();
        *n += 1;
        format!("{ID_PREFIX}-{}", *n)
    })
}

// No time crate in seer; leave `at` empty. Downstream renderers
// already treat this as best-effort per rave's precedent.
fn now_at() -> String {
    String::new()
}

pub fn emit_region(
    severity: Severity,
    region: impl Into<String>,
    title: impl Into<String>,
    why: impl Into<String>,
) -> Error {
    let error = Error {
        id: next_id(),
        severity,
        context: Context {
            surface: "seer-wasm".into(),
            region: Some(region.into()),
            anchor: None,
        },
        title: title.into(),
        why: why.into(),
        trace: vec![],
        raw: None,
        at: now_at(),
        source: Some("rust".into()),
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
    use super::*;

    #[test]
    fn emit_and_drain() {
        let _ = drain(); // clear thread-local from any prior test
        assert_eq!(count(), 0);
        emit_region(Severity::Warn, "test-region", "test title", "test why");
        emit_region(Severity::Info, "test-region", "another", "reason");
        assert_eq!(count(), 2);
        let drained = drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].context.region.as_deref(), Some("test-region"));
        assert_eq!(drained[0].severity, Severity::Warn);
        assert_eq!(count(), 0, "drain empties the ring");
    }

    #[test]
    fn id_is_monotonic_and_prefixed() {
        let a = next_id();
        let b = next_id();
        assert!(a.starts_with("err-seer-"), "id has seer prefix: {a}");
        assert!(b.starts_with("err-seer-"));
        assert_ne!(a, b, "each id is fresh");
    }
}
