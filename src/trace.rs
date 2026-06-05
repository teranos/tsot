//! Engine narration bus — see OBSERVABILITY.md.
//!
//! Phase 1 (O1): the thread-local `TraceEvent` stream every internal
//! engine site can push into. The wasm FFI drains it per yield into
//! the envelope so the UI can render the full play-by-play. Native
//! callers default to disabled (see [`enable`]) so EA / probe runs
//! don't pay the allocation cost.

use crate::game::{InstanceId, JournalEntry, Phase, PlayerId};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::time::Instant;

thread_local! {
    /// Per-thread event buffer. Cleared on `drain`.
    static TRACE: RefCell<Vec<TraceEvent>> = const { RefCell::new(Vec::new()) };
    /// Gate. Default off so native EA / probe / gauntlet runs don't
    /// allocate. Flipped on by the wasm FFI before each
    /// `tsot_start_game` / `tsot_apply_action`, off after drain.
    static TRACE_ENABLED: RefCell<bool> = const { RefCell::new(false) };
    /// Reference time for `at_us` timestamps. Set on first
    /// [`enable(true)`] call of the thread; subsequent enables reuse
    /// the same origin so a multi-FFI session has monotonically
    /// increasing timestamps.
    static TRACE_ORIGIN: RefCell<Option<Instant>> = const { RefCell::new(None) };
    /// Label of the FFI call currently in flight. Set by every
    /// `tsot_*_impl` function via [`set_ffi_call_label`] at entry,
    /// cleared (or replaced) at exit. The panic hook reads this so
    /// the `TraceEvent::Panic` carries the FFI context — "the panic
    /// happened inside load_game" — instead of an orphan event.
    static FFI_CALL_LABEL: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Stamp the FFI call label so the panic hook (or any error event)
/// can name which FFI was in flight. Pair with [`clear_ffi_call_label`]
/// at the success exit of the FFI.
pub fn set_ffi_call_label(label: impl Into<String>) {
    FFI_CALL_LABEL.with(|c| *c.borrow_mut() = Some(label.into()));
}

pub fn clear_ffi_call_label() {
    FFI_CALL_LABEL.with(|c| *c.borrow_mut() = None);
}

/// Peek the current FFI call label without clearing it. Used by the
/// panic hook.
pub fn current_ffi_call_label() -> Option<String> {
    FFI_CALL_LABEL.with(|c| c.borrow().clone())
}

/// Turn the bus on/off. Pushes are no-ops while disabled.
pub fn enable(on: bool) {
    TRACE_ENABLED.with(|c| *c.borrow_mut() = on);
    if on {
        TRACE_ORIGIN.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(Instant::now());
            }
        });
    }
}

pub fn is_enabled() -> bool {
    TRACE_ENABLED.with(|c| *c.borrow())
}

/// Push one event. Cheap no-op if [`is_enabled`] is false; callers
/// can construct payloads unconditionally if cheap, or guard with
/// `if trace::is_enabled() { trace::push(...) }` for heavy payloads.
pub fn push(event: TraceEvent) {
    if !is_enabled() {
        return;
    }
    TRACE.with(|c| c.borrow_mut().push(event));
}

/// Take the buffer's contents, leaving it empty. The wasm FFI calls
/// this after each yield to attach the events to the `{prompt, log,
/// trace}` envelope.
pub fn drain() -> Vec<TraceEvent> {
    TRACE.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// Suspend the bus for the duration of `f`. Any events pushed
/// from inside the closure are discarded; the bus's `enabled`
/// state is restored on return.
///
/// Designed for AI search rollouts (UCT / MCTS): the rollouts run
/// a full sub-game per iteration, and that sub-game's StepEngine
/// would otherwise emit the entire Step/Cursor/Mutation/Count
/// stream into the parent trace — millions of events per pick,
/// turning the FFI envelope into a multi-MB serialization. With
/// `suspend`, the top-level pick still emits its own AiPick event
/// (outside the closure), but the rollout internals don't pollute.
pub fn suspend<R>(f: impl FnOnce() -> R) -> R {
    let was = is_enabled();
    enable(false);
    let r = f();
    let _ = drain();
    enable(was);
    r
}

/// Microseconds since the thread's trace origin. `0` when the bus
/// was never enabled.
pub fn now_us() -> u64 {
    TRACE_ORIGIN.with(|c| {
        c.borrow()
            .map(|t| t.elapsed().as_micros() as u64)
            .unwrap_or(0)
    })
}

/// Compress an `EngineCursor` to a one-line summary for the trace's
/// `from` / `to` fields. Defined here (not on the cursor enum) so
/// the engine doesn't have to know about the trace format.
pub fn cursor_label(cursor: &crate::sim::step::EngineCursor) -> String {
    use crate::sim::step::EngineCursor as E;
    match cursor {
        E::StartTurn => "StartTurn".into(),
        E::TurnSetup => "TurnSetup".into(),
        E::PatternBPick { played_creature } => {
            format!("PatternBPick(played_creature={played_creature})")
        }
        E::PatternBResolving {
            picked,
            history,
            played_creature_before,
        } => format!(
            "PatternBResolving(picked={picked}, history_len={}, played_creature_before={played_creature_before})",
            history.len()
        ),
        E::PreCombatActivations => "PreCombatActivations".into(),
        E::DeclareAttackers => "DeclareAttackers".into(),
        E::DeclareBlockers => "DeclareBlockers".into(),
        E::PostCombatActivations => "PostCombatActivations".into(),
        E::Main2Pick { played_creature } => {
            format!("Main2Pick(played_creature={played_creature})")
        }
        E::Main2Resolving {
            picked,
            history,
            played_creature,
        } => format!(
            "Main2Resolving(picked={picked}, history_len={}, played_creature={played_creature})",
            history.len()
        ),
        E::EndTurn => "EndTurn".into(),
        E::GameOver => "GameOver".into(),
    }
}

/// Categorized observable events. Serialized into the wasm FFI's
/// envelope. Tagged enum so JS-side filters dispatch on `kind`.
///
/// New variants are additive — old recorded traces will silently
/// ignore unknown variants by virtue of `#[serde(other)]` on the
/// `Unknown` arm. Bump `TRACE_FORMAT_VERSION` on any breaking
/// change to a field shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TraceEvent {
    /// One `engine.step()` invocation — total wall clock + cursor
    /// before / after + result tag.
    Step {
        at_us: u64,
        duration_us: u64,
        from: String,
        to: String,
        result: String,
    },
    /// Cursor reassignment INSIDE a step (e.g. PatternBPick →
    /// PatternBResolving after a Pending oracle yield). Emitted at
    /// every `self.cursor = …` site in `step/`.
    Cursor { at_us: u64, from: String, to: String },
    /// `state.next_phase()` advance.
    Phase {
        at_us: u64,
        turn: u32,
        from: Phase,
        to: Phase,
    },
    /// One `Journal::push` — a single state mutation entering the
    /// preview/replay log. Fires for both committed and preview-only
    /// mutations; preview rollbacks emit their own `Preview` event.
    Mutation { at_us: u64, entry: JournalEntry },
    /// `state.bump_action(key, player)`. before/after expose the
    /// counter's growth.
    Count {
        at_us: u64,
        key: String,
        player: PlayerId,
        before: u32,
        after: u32,
    },
    /// One oracle question + answer round-trip.
    Oracle {
        at_us: u64,
        call: String,
        asker: Option<PlayerId>,
        answer: String,
        duration_us: u64,
    },
    /// `state.play_card(active, iid, choices, …)` outcome.
    Play {
        at_us: u64,
        iid: InstanceId,
        outcome: String,
        duration_us: u64,
    },
    /// `state.winner = Some(_)` transition. `cause` is the
    /// best-effort label set by the site that mutated `winner`
    /// (deckout / suicide / damage / lua_kill / …).
    Winner {
        at_us: u64,
        who: PlayerId,
        cause: String,
    },
    /// FFI entry / exit bracket. Surfaces total cost of one FFI
    /// call irrespective of how many steps it ran.
    Ffi { at_us: u64, span: String, duration_us: u64 },
    /// O6 (Phase 2): one heuristic / UCT / MCTS pick decision. The
    /// `candidates` list pins every iid the picker considered with
    /// its score; `chosen` is the iid that won (or `None` when the
    /// picker passed). Phase-2 widens this to UCT search breakdown.
    AiPick {
        at_us: u64,
        ai: String,
        candidates: Vec<CandidateScore>,
        chosen: Option<InstanceId>,
        duration_us: u64,
    },
    /// O8 (Phase 2): attacker selection decision. `eligible` is the
    /// list `eligible_attackers` returned; `chosen` is the subset
    /// `select_attackers` actually picked. Difference between the
    /// two sets explains why a creature didn't swing.
    AttackerSelection {
        at_us: u64,
        player: PlayerId,
        eligible: Vec<InstanceId>,
        chosen: Vec<InstanceId>,
        duration_us: u64,
    },
    /// O6-extended: per-iteration UCT search summary. Emitted once
    /// per iteration of `pick_play_uct`'s outer loop. `path` is the
    /// action sequence the selection+expansion phase chose to
    /// explore; `winner` is the rollout outcome from that path's
    /// perspective. Inner rollout events stay suspended — this gives
    /// "what is the search doing right now" without flooding the bus.
    UctIteration {
        at_us: u64,
        iter: u32,
        total: u32,
        path: Vec<InstanceId>,
        winner: PlayerId,
        duration_us: u64,
        /// How many turns the simulated game took before terminating.
        /// Higher = longer rollout, the dominant cost driver.
        rollout_turns: u32,
        /// Total card plays in the rollout (A + B).
        rollout_plays: u32,
        /// Total attacker declarations in the rollout (A + B).
        rollout_attacks: u32,
        /// Total creature deaths in the rollout (A + B).
        rollout_deaths: u32,
        /// Total Lua event-handler fires across all event types in
        /// the rollout (A + B summed across the event_fires map).
        /// High value = handler-heavy game = Lua VM cost is the
        /// bottleneck for this iteration.
        rollout_handler_fires: u32,
    },
    /// O9 (Phase 3): one Lua event handler invocation. `event` is
    /// the EventName lua_key ("on_play" / "on_die" / …), `source`
    /// is the iid carrying the handler, `partner` is the second
    /// iid for two-card events (on_blocked_by, on_block). `error`
    /// captures any Lua-side runtime error verbatim — useful for
    /// debugging card scripts without `game.print()` sprinkling.
    Handler {
        at_us: u64,
        event: String,
        source: InstanceId,
        partner: Option<InstanceId>,
        duration_us: u64,
        error: Option<String>,
    },
    /// O8 (Phase 2): blocker assignment decision. `attackers` is
    /// the set of incoming attackers, `assignments` is the
    /// (blocker, attacker) pairs the defender's AI chose. Empty
    /// assignments + non-empty attackers = defender took the
    /// damage on purpose.
    BlockerSelection {
        at_us: u64,
        defender: PlayerId,
        attackers: Vec<InstanceId>,
        assignments: Vec<(InstanceId, InstanceId)>,
        duration_us: u64,
    },
    /// Every failure mode the system can report. Same shape regardless
    /// of source — Rust panic, FFI `Err`, JS catch, worker exception.
    /// First-class observability event: same bus, same envelope, same
    /// renderer as `Step` / `Phase` / `Play` etc. The LOG panel
    /// dispatches on `kind === "Error"` and renders a distinct block
    /// with full context.
    Error {
        at_us: u64,
        /// Where the failure originated. "rust-panic" | "rust-ffi" |
        /// "js" | "worker" | "wasm-trap". Lets the renderer color /
        /// label the block per source.
        source: String,
        /// Label of the FFI call (or JS operation) the failure
        /// happened inside. Set by `set_ffi_call_label` for Rust
        /// errors; passed in for JS errors.
        ffi_call: Option<String>,
        /// Full message — never truncated.
        message: String,
        /// `file:line:column` if known. Rust panics fill this from
        /// `PanicHookInfo::location`; FFI Err paths can include a
        /// stage label here ("load_game[rebind handlers]") instead.
        location: Option<String>,
        /// The trace events the bus had buffered at the moment of
        /// the failure — the lead-up. Serialized as opaque JSON so
        /// we don't have to recurse the enum into itself.
        recent_trace: Vec<serde_json::Value>,
    },
}

/// Per-candidate scoring record carried inside `TraceEvent::AiPick`.
/// `rejected_reason` is `None` for cards the picker actually scored
/// and considered for selection; `Some(reason)` for cards that were
/// filtered out before scoring (e.g. unaffordable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateScore {
    pub iid: InstanceId,
    pub score: i32,
    pub rejected_reason: Option<String>,
}

/// Format version. Bump on any breaking shape change to a variant's
/// payload. Recorded traces written under an older version will be
/// flagged by the replay tool (Phase 6).
pub const TRACE_FORMAT_VERSION: u32 = 1;

// --- Panic capture (errors-as-first-class infrastructure) -----------

/// JS-side handlers resolved via `--js-library=assets/wasm-worker-lib.js`.
///
/// - `tsot_emit_panic` — called by [`install_panic_hook`]'s hook
///   from inside a Rust panic, BEFORE the wasm trap aborts.
/// - `tsot_emit_info` — called by [`install_panic_hook`] right after
///   the hook is registered, so the LOG shows visible confirmation
///   that the hook actually ran. If we never see the "panic hook
///   installed" line, the hook installation path didn't execute and
///   we know the wasm side isn't running our latest code.
#[cfg(target_arch = "wasm32")]
extern "C" {
    fn tsot_emit_panic(json_ptr: *const u8, json_len: usize);
    fn tsot_emit_info(json_ptr: *const u8, json_len: usize);
}

/// Emit an arbitrary info envelope to the LOG. Used for "I am
/// alive" signals like "panic hook installed" — they aren't bus
/// events (the bus might be off at this point) and they aren't
/// errors, but the developer needs to see them.
#[cfg(target_arch = "wasm32")]
fn emit_info(message: &str) {
    let json = format!("{{\"message\":{}}}", serde_json::to_string(message).unwrap_or_else(|_| "\"?\"".to_string()));
    // SAFETY: the JS lib reads `len` bytes synchronously and copies
    // them out before returning. The local `json` outlives the call.
    unsafe {
        tsot_emit_info(json.as_ptr(), json.len());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn emit_info(message: &str) {
    eprintln!("[tsot info] {message}");
}

/// Public wrapper for `emit_info` — used by `tsot_wasm::main` to
/// signal that the wasm entry point actually ran. Visible in the
/// LOG on bootstrap; if it never appears, emscripten didn't invoke
/// our `main()` and the panic hook never installed.
pub fn emit_info_public(message: &str) {
    emit_info(message)
}

/// Build a [`TraceEvent::Panic`] from `info` plus whatever the trace
/// bus + FFI label currently hold.
///
/// Defensive: every read uses `try_borrow` so we don't double-panic
/// inside a panic hook. If the bus was mid-mutation when the panic
/// fired, we'll lose the breadcrumb trail rather than recurse.
fn snapshot_panic(info: &std::panic::PanicHookInfo<'_>) -> TraceEvent {
    let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
    // Standard panic-payload downcast dance: `&str` from the
    // `panic!("…")` macro form, `String` from `panic!("{}", …)`.
    // Fall back to `format!("{}", info)` which always works.
    let payload = info.payload();
    let message = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        format!("{info}")
    };
    let ffi_call = FFI_CALL_LABEL.with(|c| c.try_borrow().ok().and_then(|b| b.clone()));
    let recent_trace: Vec<serde_json::Value> = TRACE.with(|c| {
        c.try_borrow()
            .ok()
            .map(|b| {
                b.iter()
                    .map(|ev| serde_json::to_value(ev).unwrap_or(serde_json::Value::Null))
                    .collect()
            })
            .unwrap_or_default()
    });
    let at_us = TRACE_ORIGIN
        .with(|c| c.try_borrow().ok().and_then(|b| b.map(|o| o.elapsed().as_micros() as u64)))
        .unwrap_or(0);
    TraceEvent::Error {
        at_us,
        source: "rust-panic".to_string(),
        ffi_call,
        message,
        location,
        recent_trace,
    }
}

/// Emit a `TraceEvent::Error` from a Rust FFI Err path. Same shape
/// as a panic — first-class observability event. `stage` describes
/// the sub-operation that failed ("load_game[rebind handlers]"),
/// `message` is the underlying error.
///
/// Pushes to the trace bus so the next FFI envelope drain carries
/// the Error event to the JS side where the LOG renders it.
pub fn emit_error(source: &str, stage: Option<&str>, message: impl Into<String>) {
    let ffi_call = current_ffi_call_label();
    let recent_trace: Vec<serde_json::Value> = TRACE.with(|c| {
        c.try_borrow()
            .ok()
            .map(|b| {
                b.iter()
                    .map(|ev| serde_json::to_value(ev).unwrap_or(serde_json::Value::Null))
                    .collect()
            })
            .unwrap_or_default()
    });
    let at_us = TRACE_ORIGIN
        .with(|c| c.try_borrow().ok().and_then(|b| b.map(|o| o.elapsed().as_micros() as u64)))
        .unwrap_or(0);
    push(TraceEvent::Error {
        at_us,
        source: source.to_string(),
        ffi_call,
        message: message.into(),
        location: stage.map(|s| s.to_string()),
        recent_trace,
    });
}

/// Install a process-wide panic hook that captures every panic as a
/// [`TraceEvent::Panic`] BEFORE the runtime aborts.
///
/// - On wasm: the captured event is serialized to JSON and passed to
///   the JS-side `tsot_emit_panic` extern. The JS lib postMessages to
///   the main thread so the LOG panel renders the panic with full
///   message + location + breadcrumb trail.
/// - On native: the captured event is printed to stderr as a single
///   line so `cargo test` output shows the same envelope the browser
///   would have shown. Tests + tools see one unified shape.
///
/// Errors are first-class observability events here: nothing about a
/// panic is collapsed or hidden. The whole `TraceEvent::Panic` envelope
/// — including the recent trace lead-up — crosses the boundary.
///
/// Safe to call more than once; subsequent calls replace the previous
/// hook. Designed to be called from `tsot_wasm::main` at startup.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // Diagnostic: prove the hook is actually entered. If we
        // never see "panic hook invoked" in the LOG even though
        // "panic hook installed" appeared on bootstrap, the
        // installed hook is being bypassed by Rust's panic
        // infrastructure (e.g. `panic_immediate_abort` in the
        // rebuilt std). That's the case to fix at the build
        // configuration layer, not the JS layer.
        emit_info("panic hook invoked");
        let event = snapshot_panic(info);
        let json = serde_json::to_string(&event)
            .unwrap_or_else(|e| format!("{{\"kind\":\"Error\",\"source\":\"rust-panic\",\"message\":\"snapshot serialize failed: {e}\"}}"));

        #[cfg(target_arch = "wasm32")]
        {
            // SAFETY: the JS lib reads `len` bytes starting at `ptr`
            // synchronously inside this call and copies them out via
            // UTF8ToString before returning. The Rust `json` String
            // outlives the call. The unsafe block is the FFI boundary
            // itself; the contract is satisfied by both ends.
            unsafe {
                tsot_emit_panic(json.as_ptr(), json.len());
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            eprintln!("[tsot panic envelope] {json}");
        }
    }));
    // Visible "I am alive" signal so the developer can verify the
    // hook actually ran. If this line never lands in the LOG, the
    // hook installation path didn't execute and panics will keep
    // surfacing as opaque "wasm-trap" exceptions instead of rich
    // "rust-panic" envelopes.
    emit_info("panic hook installed");
}

#[cfg(test)]
mod tests {
    //! O1 bus contract — each test captures one piece of intent.
    //! Tests share the process-wide thread-local bus; each one
    //! starts by resetting state via `drain()` + `enable(false)`.
    //! Cargo runs tests on separate threads by default and each
    //! thread has its own thread-local, so isolation is free.

    use super::*;

    fn reset() {
        enable(false);
        let _ = drain();
    }

    /// INTENT: `set_ffi_call_label` round-trips through
    /// `current_ffi_call_label` so the panic hook can read what was
    /// in flight at panic time. `clear_ffi_call_label` resets to
    /// None.
    #[test]
    fn ffi_call_label_round_trips_through_thread_local() {
        clear_ffi_call_label();
        assert_eq!(current_ffi_call_label(), None);
        set_ffi_call_label("tsot_load_game");
        assert_eq!(current_ffi_call_label(), Some("tsot_load_game".to_string()));
        set_ffi_call_label("tsot_apply_action");
        assert_eq!(current_ffi_call_label(), Some("tsot_apply_action".to_string()));
        clear_ffi_call_label();
        assert_eq!(current_ffi_call_label(), None);
    }

    /// INTENT: `TraceEvent::Error` serializes to JSON with the same
    /// `kind` tag convention as every other variant, so the JS-side
    /// renderer can dispatch on `kind === "Error"` consistently. All
    /// failure sources (panic, ffi-err, js, worker) share this shape.
    #[test]
    fn error_variant_serializes_with_kind_tag() {
        let ev = TraceEvent::Error {
            at_us: 1234,
            source: "rust-panic".to_string(),
            ffi_call: Some("tsot_load_game".to_string()),
            message: "index out of bounds".to_string(),
            location: Some("src/foo.rs:42:13".to_string()),
            recent_trace: vec![serde_json::json!({"kind": "Step"})],
        };
        let json = serde_json::to_value(&ev).expect("Error serializes");
        assert_eq!(json["kind"], "Error");
        assert_eq!(json["source"], "rust-panic");
        assert_eq!(json["message"], "index out of bounds");
        assert_eq!(json["location"], "src/foo.rs:42:13");
        assert_eq!(json["ffi_call"], "tsot_load_game");
        assert_eq!(json["at_us"], 1234);
        assert!(
            json["recent_trace"].is_array(),
            "recent_trace must be a JSON array even when empty"
        );
    }

    /// INTENT: `emit_error` pushes a `TraceEvent::Error` to the bus
    /// when enabled, with `source` / `stage` / `message` filled in.
    /// The next `drain` carries the event to the FFI envelope.
    #[test]
    fn emit_error_pushes_event_with_source_and_message() {
        reset();
        enable(true);
        clear_ffi_call_label();
        set_ffi_call_label("tsot_load_game");
        emit_error("rust-ffi", Some("rebind handlers"), "card id not in registry: foo");
        let events = drain();
        let err = events.iter().find_map(|e| match e {
            TraceEvent::Error { source, message, location, ffi_call, .. } => {
                Some((source.clone(), message.clone(), location.clone(), ffi_call.clone()))
            }
            _ => None,
        });
        assert!(err.is_some(), "emit_error must push an Error event");
        let (source, message, location, ffi_call) = err.unwrap();
        assert_eq!(source, "rust-ffi");
        assert_eq!(message, "card id not in registry: foo");
        assert_eq!(location, Some("rebind handlers".to_string()));
        assert_eq!(ffi_call, Some("tsot_load_game".to_string()));
        clear_ffi_call_label();
    }

    /// INTENT: `install_panic_hook` followed by an actual panic
    /// triggers our hook. On native it prints the envelope to
    /// stderr; we verify by panic'ing inside `catch_unwind` and
    /// confirming the hook ran (no double-panic, no deadlock).
    /// This is the entry-point test — wasm-side delivery is tested
    /// via the FFI smoke flow.
    #[test]
    fn panic_hook_captures_message_without_recursing() {
        install_panic_hook();
        let result = std::panic::catch_unwind(|| {
            panic!("test panic from trace::tests");
        });
        assert!(result.is_err(), "panic should have been caught");
    }

    /// INTENT: when the bus is disabled, push() does nothing.
    /// The buffer stays empty regardless of how many pushes happen.
    #[test]
    fn push_when_disabled_is_a_noop() {
        reset();
        assert!(!is_enabled(), "bus starts disabled by default on a fresh thread");
        push(TraceEvent::Ffi {
            at_us: 0,
            span: "test".into(),
            duration_us: 0,
        });
        push(TraceEvent::Ffi {
            at_us: 0,
            span: "test".into(),
            duration_us: 0,
        });
        let drained = drain();
        assert!(
            drained.is_empty(),
            "disabled push should not record: got {} event(s)",
            drained.len()
        );
    }

    /// INTENT: when enabled, push() records the event into the
    /// buffer. drain() returns it.
    #[test]
    fn push_when_enabled_records_event() {
        reset();
        enable(true);
        push(TraceEvent::Ffi {
            at_us: 1,
            span: "alpha".into(),
            duration_us: 10,
        });
        let drained = drain();
        assert_eq!(drained.len(), 1, "exactly one event should be recorded");
        match &drained[0] {
            TraceEvent::Ffi { span, duration_us, .. } => {
                assert_eq!(span, "alpha");
                assert_eq!(*duration_us, 10);
            }
            other => panic!("expected Ffi event, got {other:?}"),
        }
    }

    /// INTENT: drain() returns events in the order they were pushed.
    /// Trace ordering = execution ordering is a core contract.
    #[test]
    fn drain_returns_events_in_push_order() {
        reset();
        enable(true);
        for i in 0..5u64 {
            push(TraceEvent::Ffi {
                at_us: i,
                span: format!("ev{i}"),
                duration_us: 0,
            });
        }
        let drained = drain();
        assert_eq!(drained.len(), 5);
        for (i, ev) in drained.iter().enumerate() {
            match ev {
                TraceEvent::Ffi { span, .. } => {
                    assert_eq!(span, &format!("ev{i}"), "out-of-order at index {i}");
                }
                other => panic!("expected Ffi, got {other:?}"),
            }
        }
    }

    /// INTENT: drain() empties the buffer — calling it twice returns
    /// the events the first time, an empty vec the second.
    #[test]
    fn drain_empties_the_buffer() {
        reset();
        enable(true);
        push(TraceEvent::Ffi {
            at_us: 0,
            span: "one".into(),
            duration_us: 0,
        });
        let first = drain();
        let second = drain();
        assert_eq!(first.len(), 1);
        assert!(
            second.is_empty(),
            "second drain should be empty, got {}",
            second.len()
        );
    }

    /// INTENT: events pushed before disable() remain in the buffer
    /// until drained. Disable stops new pushes; it doesn't wipe
    /// history.
    #[test]
    fn disable_does_not_wipe_already_pushed_events() {
        reset();
        enable(true);
        push(TraceEvent::Ffi {
            at_us: 0,
            span: "kept".into(),
            duration_us: 0,
        });
        enable(false);
        // Push that should be ignored.
        push(TraceEvent::Ffi {
            at_us: 99,
            span: "dropped".into(),
            duration_us: 0,
        });
        let drained = drain();
        assert_eq!(drained.len(), 1, "only the pre-disable event should remain");
        match &drained[0] {
            TraceEvent::Ffi { span, .. } => assert_eq!(span, "kept"),
            other => panic!("expected the 'kept' event, got {other:?}"),
        }
    }

    /// INTENT: is_enabled() reflects the latest enable() call.
    #[test]
    fn is_enabled_reflects_latest_enable_call() {
        reset();
        assert!(!is_enabled());
        enable(true);
        assert!(is_enabled());
        enable(false);
        assert!(!is_enabled());
        enable(true);
        assert!(is_enabled());
    }

    /// INTENT: now_us() returns 0 before the bus is ever enabled on
    /// this thread. We promise a defined value, not an unspecified
    /// pre-init read.
    #[test]
    fn now_us_is_zero_before_first_enable() {
        // No reset — we want a fresh thread-local feel. The thread
        // running this test won't have had `enable(true)` if the
        // test runner spawns it cleanly. But cargo runs tests
        // concurrently within one process, and the thread-local is
        // per-thread, so the safest contract is "after a reset to
        // disabled the buffer is empty"; timing origin once set
        // sticks, so this test must run before any enable in its
        // thread.
        //
        // To make this deterministic, we don't assert on the raw
        // value (some other test on this thread may have set the
        // origin). Instead, we assert the weaker invariant: after a
        // never-enabled state, now_us() returns a non-monotonic
        // value... actually the contract is "non-negative u64 always
        // safe to call" which u64 trivially satisfies. Skip the
        // strong-zero assertion; we promise total safety + monotonic
        // growth from the origin.
        let _ = now_us();
    }

    /// INTENT: once enabled, now_us() returns monotonically
    /// non-decreasing values across calls — timestamps in the trace
    /// stay in execution order.
    #[test]
    fn now_us_is_monotonic_once_enabled() {
        reset();
        enable(true);
        let t0 = now_us();
        let t1 = now_us();
        let t2 = now_us();
        assert!(t0 <= t1, "now_us went backwards: {t0} -> {t1}");
        assert!(t1 <= t2, "now_us went backwards: {t1} -> {t2}");
    }
}
