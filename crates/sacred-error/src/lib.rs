//! Sacred Error primitive — see `ERROR.md` at the repository root.
//!
//! One typed value that crosses every layer of the system unchanged.
//! Mirror of `assets/src/Error.elm`'s `Error` record. Byte-compatible
//! serde so the JS / Elm side decodes Rust-emitted Errors with the
//! same decoder it already uses for JS-side push envelopes.
//!
//! # Scope of this crate
//!
//! This crate provides ONLY the data shape (`Error`, `Severity`,
//! `Context`, `Anchor`) plus wire-shape tests. The thread-local
//! buffer + the `emit` / `push` / `drain` / `reset` / `next_id`
//! functions live in each consumer crate's `error.rs` module, because:
//!
//! 1. Rust thread-locals can't be parameterised at runtime, so a
//!    shared bus would force every consumer to share an id namespace
//!    + clock source.
//! 2. The id-prefix (e.g. `"err-rust"` vs `"err-roam"`) and the
//!    timestamp source (`crate::trace::now_us()` in TSOT,
//!    `String::new()` in roam until its trace bus lands) DIFFER per
//!    consumer.
//! 3. Forcing per-consumer wrappers keeps the trivial mechanical
//!    code visible at each call-site's import, instead of hiding
//!    behind a macro.
//!
//! Each consumer's `error.rs` ends up ~50 lines: a `thread_local!`
//! plus thin wrappers that build [`Error`] values using THIS crate's
//! types and push them onto a local buffer.
//!
//! # The axiom (from `CLAUDE.md`)
//!
//! Errors are sacred — first-class citizens, never collapsed, dropped,
//! swallowed or suppressed. They land in front of the user,
//! contextually at points of interaction. The render path is one
//! place (`Error.view` in Elm); the wire shape is one place (this
//! crate); the layer-traversal rule is one rule (every boundary
//! preserves the typed value, never down-converts to `String`).

use serde::{Deserialize, Serialize};

/// One typed Error — same shape on the wire as `Error.elm`'s
/// `type alias Error`. Fields and field order match the Elm decoder
/// so the JSON round-trips byte-for-byte.
///
/// `id` is the stable session-scoped identifier the renderer uses
/// as the DOM key (`Html.Keyed` on the Elm side); two Errors emitted
/// from the same logical site at different times have different ids
/// so they don't collide in the rendered log.
///
/// **Source-specific fields** (`source`, `ffi_call`, `location`,
/// `js_stack`, `raw_stderr`, `requires_reload`) are the
/// previously-`LogPanel.ErrorEntry`-only payload, lifted onto the
/// canonical Error so the two parallel JS-side render paths
/// (`buildErrorBlock` + `Error.view`) can collapse to one.
/// All are optional / default-empty so existing producers don't
/// have to fill them.
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
    /// Origin classification — `"rust-panic" | "rust-ffi" | "js" |
    /// "worker" | "wasm-trap"`. Distinct from `context.surface`
    /// (which is the UI placement label): a Rust panic surfacing
    /// at the prompt has `source="rust-panic"` AND
    /// `context.surface="prompt"`. Renderers can color/tag per
    /// source independently of placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Label of the FFI call (or JS operation) the failure
    /// happened inside. Set by `set_ffi_call_label` on the Rust
    /// side for FFI errors; passed in by JS-side catches for JS
    /// errors. Lets the developer pinpoint which entry-point the
    /// failure originated from without parsing the trace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ffi_call: Option<String>,
    /// `file:line:column` for Rust panics (from
    /// `PanicHookInfo::location`) or a stage label for FFI Err
    /// paths (e.g. `"load_game[rebind handlers]"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// JS exception stack. Filled by JS-side catches that capture
    /// `err.stack`; `None` on the Rust side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub js_stack: Option<String>,
    /// Raw stderr captured from a wasm panic. Filled by the
    /// worker-side panic capture path; `None` everywhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_stderr: Option<String>,
    /// Set by rust-panic and wasm-trap sources to signal that the
    /// wasm module is dead and the developer must reload. The
    /// renderer surfaces this as a distinct footer
    /// ("reload required") so the user doesn't keep clicking into
    /// the broken state.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub requires_reload: bool,
}

/// Severity vocabulary. Matches `Error.elm`'s `Severity` decoder
/// which is case-insensitive but only accepts these four labels —
/// unknown severities FAIL decode on the Elm side (axiom enforcement,
/// no silent fallback).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
    Panic,
}

/// Where the failure happened. `surface` is required; `region` and
/// `anchor` are optional. `anchor` carries the cursor pixel position
/// when the failure was click-triggered; Rust-emitted errors leave
/// it `None` and the JS-side dispatcher fills it from the in-flight
/// `lastClickAnchor` when present.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Context {
    pub surface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

/// Pixel position where the click happened. Captured from
/// `MouseEvent.clientX/Y` on the JS side.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn severity_serializes_lowercase_strings_matching_elm_decoder() {
        // ERROR.md axiom: same wire shape on every layer. Elm's
        // decoder reads `"info" | "warn" | "error" | "panic"`
        // case-insensitively; serde rename_all = "lowercase"
        // produces the canonical lowercase form.
        assert_eq!(serde_json::to_string(&Severity::Info).unwrap(), "\"info\"");
        assert_eq!(serde_json::to_string(&Severity::Warn).unwrap(), "\"warn\"");
        assert_eq!(serde_json::to_string(&Severity::Error).unwrap(), "\"error\"");
        assert_eq!(serde_json::to_string(&Severity::Panic).unwrap(), "\"panic\"");
    }

    #[test]
    fn full_round_trip_matches_elm_wire_shape() {
        // A Rust-emitted Error serializes to the same JSON shape
        // Error.elm's decoder accepts. This is the cross-layer
        // contract — Rust pushes, JS forwards verbatim to the
        // dispatcher, Elm decodes via Error.decode. Drift here
        // breaks the sacred-error pipeline.
        let e = Error {
            id: "err-test-1".into(),
            severity: Severity::Error,
            context: Context {
                surface: "rust-ffi".into(),
                region: Some("deckbuilder".into()),
                anchor: Some(Anchor { x: 100.0, y: 50.0 }),
            },
            title: "deck preset rejected".into(),
            why: "preset[2].cards is empty".into(),
            trace: vec!["build_preset_decks".into()],
            raw: Some("{\"id\":\"x\",\"cards\":[]}".into()),
            at: "1234us".into(),
            source: None,
            ffi_call: None,
            location: None,
            js_stack: None,
            raw_stderr: None,
            requires_reload: false,
        };
        let json = serde_json::to_string(&e).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["id"], "err-test-1");
        assert_eq!(v["severity"], "error");
        assert_eq!(v["context"]["surface"], "rust-ffi");
        assert_eq!(v["context"]["region"], "deckbuilder");
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
    fn ffi_fields_round_trip_when_populated() {
        // Source-specific fields (the LogPanel.ErrorEntry-only set
        // before unification) round-trip cleanly. Filled by Rust
        // panic hooks / FFI envelopes / JS catches; renderer reads
        // them via the same decoder.
        let e = Error {
            id: "err-panic-1".into(),
            severity: Severity::Panic,
            context: Context {
                surface: "prompt".into(),
                region: None,
                anchor: None,
            },
            title: "wasm trapped".into(),
            why: "panicked at lib.rs:42:8".into(),
            trace: vec![],
            raw: None,
            at: "9876us".into(),
            source: Some("rust-panic".into()),
            ffi_call: Some("tsot_apply_action".into()),
            location: Some("src/lib.rs:42:8".into()),
            js_stack: None,
            raw_stderr: Some("thread main panicked\n".into()),
            requires_reload: true,
        };
        let json = serde_json::to_string(&e).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["source"], "rust-panic");
        assert_eq!(v["ffi_call"], "tsot_apply_action");
        assert_eq!(v["location"], "src/lib.rs:42:8");
        assert_eq!(v["raw_stderr"], "thread main panicked\n");
        assert_eq!(v["requires_reload"], true);
        assert!(v.get("js_stack").is_none(), "None field omitted");
        let round: Error = serde_json::from_value(v).unwrap();
        assert_eq!(round, e);
    }

    #[test]
    fn optional_fields_omitted_when_default() {
        // Empty trace + None raw + None region/anchor must not
        // appear in the serialized JSON. Elm's optionalField
        // decoders accept their absence and supply defaults. The
        // FFI-specific fields (added 2026-06-18) follow the same
        // rule: producers that don't fill them omit them.
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
            source: None,
            ffi_call: None,
            location: None,
            js_stack: None,
            raw_stderr: None,
            requires_reload: false,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("\"trace\""), "trace should be omitted: {json}");
        assert!(!json.contains("\"raw\""), "raw should be omitted: {json}");
        assert!(!json.contains("\"region\""), "region should be omitted: {json}");
        assert!(!json.contains("\"anchor\""), "anchor should be omitted: {json}");
        assert!(!json.contains("\"source\""), "source omitted when None: {json}");
        assert!(!json.contains("\"ffi_call\""), "ffi_call omitted: {json}");
        assert!(!json.contains("\"location\""), "location omitted: {json}");
        assert!(!json.contains("\"js_stack\""), "js_stack omitted: {json}");
        assert!(!json.contains("\"raw_stderr\""), "raw_stderr omitted: {json}");
        assert!(!json.contains("\"requires_reload\""), "requires_reload omitted when false: {json}");
    }

    #[test]
    fn unknown_severity_label_fails_decode() {
        // ERROR.md Slice 1 axiom-enforcement test: the four severity
        // labels are a closed set. An unknown label is a contract
        // violation that must FAIL decode — never silently downgrade
        // to a default Info.
        let bad = "\"unknown\"";
        let result: Result<Severity, _> = serde_json::from_str(bad);
        assert!(
            result.is_err(),
            "unknown severity label must fail decode (axiom): got Ok({:?})",
            result.ok(),
        );
    }
}
