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
    fn optional_fields_omitted_when_default() {
        // Empty trace + None raw + None region/anchor must not
        // appear in the serialized JSON. Elm's optionalField
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
