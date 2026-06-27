//! rave's Sacred Error bus. Mirrors roam/src/error.rs.
//!
//! Types live in `sacred-error` (`crates/sacred-error/src/lib.rs`).
//! This module owns only the rave-specific bits:
//!   - thread_local buffer
//!   - `"err-rave-N"` id namespace
//!   - placeholder timestamp source
//!
//! Per ERROR.md, this exists so I (and any developer / Claude session)
//! can SEE failures with origin context — surface, region, title, why —
//! instead of grepping wasm-bindgen function indices.

pub use sacred_error::{Anchor, Context, Error, Severity};

use std::cell::RefCell;

const ID_PREFIX: &str = "err-rave";

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

pub fn next_id() -> String {
    ERROR_COUNTER.with(|c| {
        let mut n = c.borrow_mut();
        *n += 1;
        format!("{ID_PREFIX}-{}", *n)
    })
}

#[cfg(target_arch = "wasm32")]
fn now_at() -> String {
    format!("{}ms", js_sys::Date::now() as u64)
}

#[cfg(not(target_arch = "wasm32"))]
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
            surface: "rave-wasm".into(),
            region: Some(region.into()),
            anchor: None,
        },
        title: title.into(),
        why: why.into(),
        trace: vec![],
        raw: None,
        at: now_at(),
        source: Some("rust-ffi".into()),
        ffi_call: None,
        location: None,
        js_stack: None,
        raw_stderr: None,
        requires_reload: false,
    };
    push(error.clone());
    error
}
