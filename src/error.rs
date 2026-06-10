//! Sacred Error primitive — see ERROR.md.
//!
//! Mirror of `assets/src/Error.elm`'s `Error` record. Byte-compatible
//! serde so the JS / Elm side decodes Rust-emitted Errors with the
//! same `Error.decode` it already uses for JS-side `tsotPushError`
//! envelopes. One type, one shape, one decoder — across every layer.
//!
//! Thread-local buffer pattern mirrors `crate::trace`: the wasm FFI
//! enables the bus around each call, every Rust site can `emit(...)`
//! a typed Error, the FFI drains the buffer into the envelope's
//! `errors: Vec<Error>` field. JS-side dispatcher routes each
//! through `tsotPushError` so the overlay lands at the cursor (or
//! at its surface fallback when no cursor is in flight).
//!
//! Per the axiom in `ERROR.md`: **No `eprintln!`-and-continue. No
//! `Result` swallow. Every failure becomes one of these or it
//! doesn't exist.**

use serde::{Deserialize, Serialize};
use std::cell::RefCell;

/// One typed Error — same shape on the wire as `Error.elm`'s
/// `type alias Error`. Fields and field order match the Elm
/// decoder so the JSON round-trips byte-for-byte.
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

/// Severity vocabulary. Matches `Error.elm`'s `Severity` decoder
/// which is case-insensitive but only accepts these four labels —
/// unknown severities FAIL decode (axiom enforcement).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
    Panic,
}

/// Where the failure happened. `surface` is required, region +
/// anchor optional. Anchor carries the cursor pixel position when
/// the failure was click-triggered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Context {
    pub surface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

/// Pixel position where the click happened. Captured from
/// `MouseEvent.clientX/Y` on the JS side; absent for Rust-emitted
/// errors (which can't see the cursor) — JS-side dispatcher fills
/// it from the in-flight `lastClickAnchor` when present.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
}

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
        format!("err-rust-{}", *n)
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
        // ERROR.md axiom: same wire shape on every layer.
        // Elm's decoder reads `"info" | "warn" | "error" | "panic"`
        // case-insensitively; serde rename_all = "lowercase" produces
        // the canonical lowercase form.
        assert_eq!(serde_json::to_string(&Severity::Info).unwrap(), "\"info\"");
        assert_eq!(serde_json::to_string(&Severity::Warn).unwrap(), "\"warn\"");
        assert_eq!(
            serde_json::to_string(&Severity::Error).unwrap(),
            "\"error\""
        );
        assert_eq!(
            serde_json::to_string(&Severity::Panic).unwrap(),
            "\"panic\""
        );
    }

    #[test]
    fn full_round_trip_matches_elm_wire_shape() {
        // INTENT: A Rust-emitted Error serializes to the same JSON
        // shape Error.elm's decoder accepts. This is the cross-
        // layer contract — Rust pushes, JS forwards verbatim to
        // tsotPushError, Elm decodes via Error.decode. Drift here
        // breaks the sacred-error pipeline.
        let e = Error {
            id: "err-rust-1".into(),
            severity: Severity::Error,
            context: Context {
                surface: "rust-ffi".into(),
                region: Some("tsot_list_preset_decks".into()),
                anchor: Some(Anchor { x: 100.0, y: 50.0 }),
            },
            title: "deck preset rejected".into(),
            why: "preset[2].cards is empty".into(),
            trace: vec!["build_preset_decks".into()],
            raw: Some("{\"id\":\"x\",\"cards\":[]}".into()),
            at: "1234us".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["id"], "err-rust-1");
        assert_eq!(v["severity"], "error");
        assert_eq!(v["context"]["surface"], "rust-ffi");
        assert_eq!(v["context"]["region"], "tsot_list_preset_decks");
        assert_eq!(v["context"]["anchor"]["x"], 100.0);
        assert_eq!(v["context"]["anchor"]["y"], 50.0);
        assert_eq!(v["title"], "deck preset rejected");
        assert_eq!(v["why"], "preset[2].cards is empty");
        assert_eq!(v["trace"].as_array().unwrap().len(), 1);
        assert_eq!(v["raw"], "{\"id\":\"x\",\"cards\":[]}");
        assert_eq!(v["at"], "1234us");

        let round: Error = serde_json::from_value(v).unwrap();
        assert_eq!(round, e);
    }

    #[test]
    fn optional_fields_omitted_when_default() {
        // INTENT: empty trace + None raw + None region/anchor must
        // not appear in the serialized JSON. Elm's optionalField
        // decoders accept their absence and supply defaults.
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
            at: "0us".into(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("\"trace\""), "trace should be omitted: {json}");
        assert!(!json.contains("\"raw\""), "raw should be omitted: {json}");
        assert!(!json.contains("\"region\""), "region should be omitted: {json}");
        assert!(!json.contains("\"anchor\""), "anchor should be omitted: {json}");
    }

    #[test]
    fn emit_pushes_to_buffer_and_returns_the_error() {
        // Isolate this test's buffer state. The thread-local is
        // shared, so other tests' emits could leak in — drain first.
        reset();
        let e = emit(
            Severity::Warn,
            "test-surface",
            "test title",
            "test why",
        );
        let drained = drain();
        assert_eq!(drained.len(), 1, "buffer should have one entry");
        assert_eq!(drained[0], e);
        assert_eq!(drained[0].context.surface, "test-surface");
        assert_eq!(drained[0].severity, Severity::Warn);

        // After drain, buffer is empty.
        assert_eq!(drain().len(), 0, "drain should leave buffer empty");
    }

    #[test]
    fn next_id_is_monotonic_within_a_session() {
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
