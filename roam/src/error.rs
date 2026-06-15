//! Sacred Error primitive — see ERROR.md (at TSOT root).
//!
//! Mirror of `assets/src/Error.elm`'s `Error` record. Byte-compatible
//! serde so the JS / Elm side decodes Rust-emitted Errors with the
//! same `Error.decode` it already uses for JS-side `roamPushError`
//! envelopes.
//!
//! CONVERGENCE NOTE: roam's local copy. TSOT's `src/error.rs` carries
//! the canonical implementation upstream.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Error {
    pub id: String,
    pub severity: Severity,
    pub context: Context,
    pub title: String,
    pub why: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    pub at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
    Panic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Context {
    pub surface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
}

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
        format!("err-roam-{}", *n)
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
        // TODO: wire wall-clock timestamp. js-sys::Date::now() on wasm,
        // std::time::SystemTime on native. Empty for now is fine — the
        // Elm decoder accepts any string.
        at: String::new(),
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
    };
    push(error.clone());
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn severity_serializes_lowercase_strings_matching_elm_decoder() {
        assert_eq!(serde_json::to_string(&Severity::Info).unwrap(), "\"info\"");
        assert_eq!(serde_json::to_string(&Severity::Warn).unwrap(), "\"warn\"");
        assert_eq!(serde_json::to_string(&Severity::Error).unwrap(), "\"error\"");
        assert_eq!(serde_json::to_string(&Severity::Panic).unwrap(), "\"panic\"");
    }

    #[test]
    fn full_round_trip_matches_elm_wire_shape() {
        let e = Error {
            id: "err-roam-1".into(),
            severity: Severity::Error,
            context: Context {
                surface: "rust-ffi".into(),
                region: Some("roam_tick".into()),
                anchor: Some(Anchor { x: 100.0, y: 50.0 }),
            },
            title: "bad input".into(),
            why: "dt_ms negative".into(),
            trace: vec!["roam_tick_impl".into()],
            raw: Some("{\"dt_ms\":-1}".into()),
            at: "1234us".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["id"], "err-roam-1");
        assert_eq!(v["severity"], "error");
        assert_eq!(v["context"]["surface"], "rust-ffi");
        assert_eq!(v["context"]["region"], "roam_tick");
        assert_eq!(v["context"]["anchor"]["x"], 100.0);
        assert_eq!(v["context"]["anchor"]["y"], 50.0);
        let round: Error = serde_json::from_value(v).unwrap();
        assert_eq!(round, e);
    }

    #[test]
    fn optional_fields_omitted_when_default() {
        let e = Error {
            id: "x".into(),
            severity: Severity::Info,
            context: Context {
                surface: "test".into(),
                region: None,
                anchor: None,
            },
            title: "t".into(),
            why: "w".into(),
            trace: vec![],
            raw: None,
            at: String::new(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("\"trace\""), "trace should be omitted: {json}");
        assert!(!json.contains("\"raw\""), "raw should be omitted: {json}");
        assert!(!json.contains("\"region\""), "region should be omitted: {json}");
        assert!(!json.contains("\"anchor\""), "anchor should be omitted: {json}");
    }

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
    fn next_id_is_monotonic_within_a_session() {
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
