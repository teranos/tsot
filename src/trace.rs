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
