//! O2 contract — Step + Cursor + Phase events emitted by `engine.step()`.
//!
//! Each test captures one piece of intent about what the engine
//! narration should expose. Tests are isolated by the thread-local
//! trace bus: each test resets via `trace::enable(false) + drain()`
//! at entry.

use super::*;
use crate::card::CardType;
use crate::cast_routing::CastRouting;
use crate::trace::{self, TraceEvent};

/// Reset the trace bus + reopen it. Each test starts from a known
/// state so events from prior tests can't bleed in.
fn fresh_trace() {
    trace::enable(false);
    let _ = trace::drain();
    trace::enable(true);
}

/// Build a minimal AI-vs-AI engine. Vanilla creature mirror — same
/// fixture the existing `step_engine_constructs_at_start_turn` test
/// uses, kept local to avoid cross-test coupling.
fn make_engine() -> StepEngine {
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let template = registry
        .cards()
        .iter()
        .find(|c| {
            matches!(c.kind, CardType::Creature)
                && c.handlers.is_empty()
                && c.kind.is_castable()
        })
        .unwrap()
        .clone();
    let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
    let deck_b = deck_a.clone();
    let state = GameState::new(deck_a, deck_b);
    StepEngine::new(
        state,
        [AiKind::Heuristic, AiKind::Heuristic],
        registry,
        0xCAFE,
    )
}

/// INTENT: every `engine.step()` call produces exactly one Step
/// event. The Step event is the "summary" record per step
/// invocation, regardless of how many sub-transitions happened.
#[test]
fn step_emits_exactly_one_step_event() {
    fresh_trace();
    let mut engine = make_engine();
    let _ = engine.step(None);
    let events = trace::drain();
    let step_count = events
        .iter()
        .filter(|e| matches!(e, TraceEvent::Step { .. }))
        .count();
    assert_eq!(
        step_count, 1,
        "expected exactly 1 Step event, got {step_count}; full events: {events:#?}"
    );
}

/// INTENT: the Step event's `from` field equals the cursor label
/// before the step ran, and `to` equals the cursor label after.
/// This is what makes the Step event the per-call "what changed"
/// summary.
#[test]
fn step_event_records_from_and_to_cursor_labels() {
    fresh_trace();
    let mut engine = make_engine();
    let before = trace::cursor_label(&engine.cursor);
    let _ = engine.step(None);
    let after = trace::cursor_label(&engine.cursor);
    let events = trace::drain();
    let (from, to) = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Step { from, to, .. } => Some((from.clone(), to.clone())),
            _ => None,
        })
        .expect("Step event present in drained trace");
    assert_eq!(from, before, "Step.from should equal pre-step cursor label");
    assert_eq!(to, after, "Step.to should equal post-step cursor label");
}

/// INTENT: the Step event's `result` field tags the StepResult
/// variant returned. For a vanilla AI game, the first step from
/// StartTurn returns Continue.
#[test]
fn step_event_records_continue_result() {
    fresh_trace();
    let mut engine = make_engine();
    let _ = engine.step(None);
    let events = trace::drain();
    let result = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Step { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("Step event present");
    assert_eq!(
        result, "Continue",
        "first step from StartTurn should report Continue, got {result:?}"
    );
}

/// INTENT: when the cursor changes inside a step (e.g. StartTurn →
/// TurnSetup), at least one Cursor event is recorded. The Cursor
/// event records the exact transition irrespective of whether the
/// step kept going.
#[test]
fn cursor_change_inside_step_emits_cursor_event() {
    fresh_trace();
    let mut engine = make_engine();
    let _ = engine.step(None);
    let events = trace::drain();
    let cursor_count = events
        .iter()
        .filter(|e| matches!(e, TraceEvent::Cursor { .. }))
        .count();
    assert!(
        cursor_count >= 1,
        "expected ≥1 Cursor event after a step that advances the cursor, got {cursor_count}; events: {events:#?}"
    );
}

/// INTENT: Cursor events fire AT the moment of each `self.cursor
/// = …` assignment, which is BEFORE the Step event is recorded at
/// the end of the step. So in the drained stream, cursor events
/// precede the step event for the same step call.
#[test]
fn cursor_events_precede_step_event() {
    fresh_trace();
    let mut engine = make_engine();
    let _ = engine.step(None);
    let events = trace::drain();
    let step_idx = events
        .iter()
        .position(|e| matches!(e, TraceEvent::Step { .. }))
        .expect("Step event present");
    let cursor_idx = events
        .iter()
        .position(|e| matches!(e, TraceEvent::Cursor { .. }))
        .expect("Cursor event present");
    assert!(
        cursor_idx < step_idx,
        "Cursor event index ({cursor_idx}) should be < Step event index ({step_idx})"
    );
}

/// INTENT: when `state.next_phase()` advances the game phase, a
/// Phase event records (turn, from-phase, to-phase). TurnSetup
/// walks the engine through Untap → Draw → Main1 so several Phase
/// events fire across the first few steps.
#[test]
fn phase_advance_emits_phase_events() {
    fresh_trace();
    let mut engine = make_engine();
    // Drive a handful of steps so TurnSetup advances through phases.
    for _ in 0..6 {
        match engine.step(None) {
            StepResult::Continue => {}
            _ => break,
        }
    }
    let events = trace::drain();
    let phase_count = events
        .iter()
        .filter(|e| matches!(e, TraceEvent::Phase { .. }))
        .count();
    assert!(
        phase_count >= 1,
        "expected ≥1 Phase event during TurnSetup, got {phase_count}; events: {events:#?}"
    );
}

/// INTENT: when the bus is disabled, `engine.step()` emits zero
/// events. The instrumentation respects the gate; native sim
/// callers (EA, gauntlets) that never enable trace pay zero
/// allocation cost.
#[test]
fn step_emits_no_events_when_trace_disabled() {
    trace::enable(false);
    let _ = trace::drain();
    let mut engine = make_engine();
    let _ = engine.step(None);
    let events = trace::drain();
    assert!(
        events.is_empty(),
        "trace disabled → step should not push events, got {events:#?}"
    );
}

// ----- O4: Oracle events --------------------------------------

/// INTENT: every `choose_card` invocation on `HumanReplayOracle`
/// emits an `Oracle` event tagged `call = "choose_card"` with the
/// asker carried through.
#[test]
fn oracle_choose_card_emits_oracle_event_with_choose_card_tag() {
    use crate::choice::{ChoiceOracle, ChooseCardRequest, RandomOracle};
    use crate::sim::human::HumanReplayOracle;
    use rand::SeedableRng;
    fresh_trace();
    let state = GameState::new(Vec::new(), Vec::new());
    let mut oracle =
        HumanReplayOracle::new(RandomOracle::new(StdRng::seed_from_u64(0)), None);
    let req = ChooseCardRequest {
        pool: Vec::new(),
        asker: Some(PlayerId::A),
        host: None,
        optional: true,
        prompt: String::new(),
    };
    let _ = oracle.choose_card(&state, req);
    let events = trace::drain();
    let (call, asker) = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Oracle { call, asker, .. } => Some((call.clone(), *asker)),
            _ => None,
        })
        .expect("Oracle event present");
    assert_eq!(call, "choose_card");
    assert_eq!(asker, Some(PlayerId::A));
}

/// INTENT: `confirm` calls emit Oracle events tagged
/// `call = "confirm"` with the asker carried.
#[test]
fn oracle_confirm_emits_oracle_event_with_confirm_tag() {
    use crate::choice::{ChoiceOracle, RandomOracle};
    use crate::sim::human::HumanReplayOracle;
    use rand::SeedableRng;
    fresh_trace();
    let state = GameState::new(Vec::new(), Vec::new());
    let mut oracle =
        HumanReplayOracle::new(RandomOracle::new(StdRng::seed_from_u64(0)), None);
    let _ = oracle.confirm(&state, PlayerId::B, "test");
    let events = trace::drain();
    let (call, asker) = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Oracle { call, asker, .. } => Some((call.clone(), *asker)),
            _ => None,
        })
        .expect("Oracle event present");
    assert_eq!(call, "confirm");
    assert_eq!(asker, Some(PlayerId::B));
}

/// INTENT: `choose_player` calls emit `call = "choose_player"`.
#[test]
fn oracle_choose_player_emits_oracle_event_with_choose_player_tag() {
    use crate::choice::{ChoiceOracle, ChoosePlayerRequest, RandomOracle};
    use crate::sim::human::HumanReplayOracle;
    use rand::SeedableRng;
    fresh_trace();
    let state = GameState::new(Vec::new(), Vec::new());
    let mut oracle =
        HumanReplayOracle::new(RandomOracle::new(StdRng::seed_from_u64(0)), None);
    let req = ChoosePlayerRequest {
        exclude: Vec::new(),
        optional: true,
        prompt: String::new(),
    };
    let _ = oracle.choose_player(&state, req);
    let events = trace::drain();
    let call = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Oracle { call, .. } => Some(call.clone()),
            _ => None,
        })
        .expect("Oracle event present");
    assert_eq!(call, "choose_player");
}

/// INTENT: `choose_int` calls emit `call = "choose_int"`.
#[test]
fn oracle_choose_int_emits_oracle_event_with_choose_int_tag() {
    use crate::choice::{ChoiceOracle, ChooseIntRequest, RandomOracle};
    use crate::sim::human::HumanReplayOracle;
    use rand::SeedableRng;
    fresh_trace();
    let state = GameState::new(Vec::new(), Vec::new());
    let mut oracle =
        HumanReplayOracle::new(RandomOracle::new(StdRng::seed_from_u64(0)), None);
    let req = ChooseIntRequest {
        min: 0,
        max: 5,
        prompt: String::new(),
    };
    let _ = oracle.choose_int(&state, req);
    let events = trace::drain();
    let call = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Oracle { call, .. } => Some(call.clone()),
            _ => None,
        })
        .expect("Oracle event present");
    assert_eq!(call, "choose_int");
}

/// INTENT: when the bus is disabled, oracle methods emit no events.
#[test]
fn oracle_emits_no_events_when_trace_disabled() {
    use crate::choice::{ChoiceOracle, ChooseCardRequest, RandomOracle};
    use crate::sim::human::HumanReplayOracle;
    use rand::SeedableRng;
    trace::enable(false);
    let _ = trace::drain();
    let state = GameState::new(Vec::new(), Vec::new());
    let mut oracle =
        HumanReplayOracle::new(RandomOracle::new(StdRng::seed_from_u64(0)), None);
    let req = ChooseCardRequest {
        pool: Vec::new(),
        asker: Some(PlayerId::A),
        host: None,
        optional: true,
        prompt: String::new(),
    };
    let _ = oracle.choose_card(&state, req);
    assert!(trace::drain().is_empty());
}
