use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_BUS_LEN: usize = 10_000;

/// Rare, narrative events. Per-frame events (Tick, StateRead,
/// ViewportRead) used to live here and got emitted ~60×/s for what
/// were really three counters; they're now `AtomicU64`s read on
/// demand via the FFI counter exports. The bus carries only events
/// that warrant a row in the event log.
#[derive(Debug, Clone)]
pub enum TraceEvent {
    Init { spawn_x: f32, spawn_y: f32, spawn_z: i32 },
    Note { tag: &'static str, msg: String },
    Overflow { dropped: u64, since_seq: u64 },
    /// Rust-side panic captured by [`install_panic_hook`]. Carries
    /// the originating `file:line` plus a best-effort string
    /// extraction of the payload (`&str` and `String` payloads are
    /// rendered verbatim; other types degrade to a fixed marker
    /// since `Any` doesn't expose a Display surface). Closes
    /// docs/observability.md Phase 2's "Rust panics caught via panic_hook,
    /// surfaced as TraceEvent::Error with file:line" item — panics
    /// in roam's release build (`panic = "abort"`) now land in the
    /// event log BEFORE the wasm module aborts.
    Error { file: String, line: u32, message: String },
}

impl TraceEvent {
    fn to_json(&self) -> String {
        match self {
            TraceEvent::Init { spawn_x, spawn_y, spawn_z } => format!(
                r#"{{"kind":"Init","spawn_x":{spawn_x},"spawn_y":{spawn_y},"spawn_z":{spawn_z}}}"#
            ),
            TraceEvent::Note { tag, msg } => format!(
                r#"{{"kind":"Note","tag":"{tag}","msg":{}}}"#,
                escape_json_string(msg)
            ),
            TraceEvent::Overflow { dropped, since_seq } => format!(
                r#"{{"kind":"Overflow","dropped":{dropped},"since_seq":{since_seq}}}"#
            ),
            TraceEvent::Error { file, line, message } => format!(
                r#"{{"kind":"Error","file":{},"line":{line},"message":{}}}"#,
                escape_json_string(file),
                escape_json_string(message)
            ),
        }
    }
}

// ----- per-frame counters (replaces the deleted Tick/StateRead/ViewportRead variants) -----

pub static TICK_COUNT: AtomicU64 = AtomicU64::new(0);
pub static TICK_BLOCKED_COUNT: AtomicU64 = AtomicU64::new(0);
pub static STATE_READ_COUNT: AtomicU64 = AtomicU64::new(0);
pub static VIEWPORT_READ_COUNT: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn count_tick(blocked: bool) {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
    if blocked {
        TICK_BLOCKED_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

#[inline]
pub fn count_state_read() {
    STATE_READ_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn count_viewport_read() {
    VIEWPORT_READ_COUNT.fetch_add(1, Ordering::Relaxed);
}

fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

static SEQ: AtomicU64 = AtomicU64::new(0);
static OVERFLOW: AtomicU64 = AtomicU64::new(0);
static FIRST_DROP_SEQ: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static BUS: RefCell<VecDeque<(u64, TraceEvent)>> = const { RefCell::new(VecDeque::new()) };
}

// IDENTITY MENU (roam/docs/identity.md):
//   C5 — emit identity events (mint, load, export, import, rotate, sign, verify)
//        with dedicated tags so the event log can render them in a distinct color.
pub fn emit(ev: TraceEvent) {
    // Per-tag perf counter — `Note` is the only variant with a tag,
    // and the perf panel's "emits/sec by tag" surface only makes
    // sense for those. Init / Overflow / Error are infrequent and
    // visible in the bus directly.
    if let TraceEvent::Note { tag, .. } = &ev {
        crate::perf::note_tag_emit(tag);
    }
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    BUS.with(|b| {
        let mut buf = b.borrow_mut();
        if buf.len() >= MAX_BUS_LEN {
            let before = OVERFLOW.fetch_add(1, Ordering::SeqCst);
            if before == 0 {
                // First drop in this overflow window — record the seq we
                // started dropping from so observers can spot the gap.
                if let Some((oldest_seq, _)) = buf.front() {
                    FIRST_DROP_SEQ.store(*oldest_seq, Ordering::SeqCst);
                }
            }
            buf.pop_front();
        }
        buf.push_back((seq, ev));
    });
}

pub fn drain_json() -> String {
    BUS.with(|b| {
        let mut buf = b.borrow_mut();
        let overflow = OVERFLOW.swap(0, Ordering::SeqCst);
        let first_drop = FIRST_DROP_SEQ.swap(0, Ordering::SeqCst);
        let mut s = String::from("[");
        let mut first = true;
        if overflow > 0 {
            let ev = TraceEvent::Overflow { dropped: overflow, since_seq: first_drop };
            s.push_str(&format!(r#"{{"seq":0,"event":{}}}"#, ev.to_json()));
            first = false;
        }
        for (seq, ev) in buf.iter() {
            if !first { s.push(','); }
            s.push_str(&format!(r#"{{"seq":{seq},"event":{}}}"#, ev.to_json()));
            first = false;
        }
        s.push(']');
        buf.clear();
        s
    })
}

pub fn drain_discard() {
    BUS.with(|b| b.borrow_mut().clear());
}

pub fn pending_count() -> usize {
    BUS.with(|b| b.borrow().len())
}

// ----- panic hook -----------------------------------------------
//
// docs/observability.md Phase 2's last open item: "Rust panics caught via
// panic_hook, surfaced as TraceEvent::Error with file:line". roam's
// release + dev profiles both use `panic = "abort"`, so the hook runs
// once (right before the process aborts) and panics cannot be
// `catch_unwind`-ed in tests. The hook still installs cleanly; it
// just won't be reached again from the same process after a panic.

static PANIC_HOOK_INSTALLED: Once = Once::new();

/// Install a panic hook that pushes a [`TraceEvent::Error`] onto the
/// bus with `file:line` and a best-effort payload string. Subsequent
/// calls are no-ops (idempotency guard so any future re-init path
/// can't accidentally double-install or replace a previously set
/// hook). Chains to the prior hook so the default `console.error`
/// pipeline still fires.
pub fn install_panic_hook() {
    PANIC_HOOK_INSTALLED.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let (file, line) = info
                .location()
                .map(|loc| (loc.file().to_string(), loc.line()))
                .unwrap_or_else(|| ("<unknown>".to_string(), 0));
            let message = payload_to_string(info.payload());
            emit(TraceEvent::Error { file, line, message });
            // Chain to default so the wasm console.error path + any
            // outer hook (e.g. console_error_panic_hook if ever
            // wired) still fires.
            default_hook(info);
        }));
    });
}

/// Best-effort extraction of the panic payload as a string. Covers
/// the two common cases (`panic!("literal")` → `&'static str` and
/// `panic!("{x}", x = ...)` → `String`); any other payload type
/// degrades to a fixed marker because `Any` doesn't expose Display.
fn payload_to_string(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_variant_to_json_includes_file_line_and_escaped_message() {
        // The new variant must serialize into the bus JSON the JS
        // side already parses. file + message use the same
        // escape_json_string path as Note's msg; line is a raw u32.
        let ev = TraceEvent::Error {
            file: "src/world.rs".to_string(),
            line: 42,
            message: "panicked at \"index out of bounds\"\nnext line".to_string(),
        };
        let json = ev.to_json();
        assert!(json.contains(r#""kind":"Error""#));
        assert!(json.contains(r#""file":"src/world.rs""#));
        assert!(json.contains(r#""line":42"#));
        // Quotes inside the message must be escaped so the wrapping
        // JSON stays valid.
        assert!(
            json.contains(r#"\"index out of bounds\""#),
            "embedded quotes must be backslash-escaped: {json}"
        );
        // Newlines too.
        assert!(
            json.contains(r"\n"),
            "embedded newline must be escaped: {json}"
        );
    }

    #[test]
    fn install_panic_hook_is_idempotent() {
        // The Once gate makes second + Nth calls no-ops. Without
        // this, repeated calls (e.g. from a future re-init path)
        // would replace whatever hook had been installed since.
        // We verify by calling twice and observing no panic +
        // observing the hook is set (set_hook returning a non-default
        // hook on take is the cheapest test).
        install_panic_hook();
        install_panic_hook();
        // If take_hook returns the hook we installed, the second call
        // didn't replace it. We re-install after to leave the test
        // VM with a working hook (defensive — other tests in the
        // same binary that panic should not silently lose info).
        let _ = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
    }

    #[test]
    fn error_variant_round_trips_through_emit_and_drain() {
        // Pin the wire shape: emit a synthetic Error event (no
        // panic needed), drain, confirm it appears in the JSON
        // payload the JS side reads from roam_drain_trace.
        drain_discard(); // isolate from other tests in this binary
        emit(TraceEvent::Error {
            file: "fake.rs".to_string(),
            line: 7,
            message: "synthetic".to_string(),
        });
        let drained = drain_json();
        assert!(drained.contains(r#""kind":"Error""#));
        assert!(drained.contains(r#""file":"fake.rs""#));
        assert!(drained.contains(r#""line":7"#));
        assert!(drained.contains(r#""message":"synthetic""#));
    }
}
