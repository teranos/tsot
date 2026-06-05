//! O3 + O4 contract — Mutation (via `Journal::push`) + Count (via
//! `GameState::bump_action`) + Play (via `GameState::play_card`) +
//! Winner (via `GameState::set_winner`) events. Tests are
//! independent of the step engine: a fresh `Journal` or `GameState`
//! is enough to exercise the instrumentation.

use super::*;
use crate::trace::{self, TraceEvent};
use crate::game::PlayChoices;

fn fresh_trace() {
    trace::enable(false);
    let _ = trace::drain();
    trace::enable(true);
}

fn empty_state() -> GameState {
    GameState::new(Vec::new(), Vec::new())
}

/// INTENT: every `Journal::push` records a `TraceEvent::Mutation`
/// with the same `JournalEntry` payload. The journal entry IS the
/// payload — preview rollback (Phase 4 work) emits its own event
/// separately.
#[test]
fn journal_push_emits_mutation_event_with_matching_entry() {
    fresh_trace();
    let mut journal = Journal::new();
    journal.push(JournalEntry::SetTapped {
        iid: "TEST:0001".to_string(),
        was: false,
        now: true,
    });
    let events = trace::drain();
    let mutation = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Mutation { entry, .. } => Some(entry.clone()),
            _ => None,
        })
        .expect("Mutation event present");
    match mutation {
        JournalEntry::SetTapped { iid, was, now } => {
            assert_eq!(iid, "TEST:0001");
            assert!(!was);
            assert!(now);
        }
        other => panic!("expected SetTapped entry, got {other:?}"),
    }
}

/// INTENT: a sequence of journal pushes produces Mutation events
/// in the order the pushes happened. The bus is FIFO; ordering is
/// the contract that lets a replay tool reconstruct execution.
#[test]
fn multiple_pushes_record_mutation_events_in_order() {
    fresh_trace();
    let mut journal = Journal::new();
    for i in 0..4 {
        journal.push(JournalEntry::SetTapped {
            iid: format!("X:{i:04}"),
            was: false,
            now: true,
        });
    }
    let events = trace::drain();
    let mutations: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            TraceEvent::Mutation {
                entry: JournalEntry::SetTapped { iid, .. },
                ..
            } => Some(iid.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(mutations.len(), 4);
    for (i, iid) in mutations.iter().enumerate() {
        assert_eq!(iid, &format!("X:{i:04}"));
    }
}

/// INTENT: when the bus is disabled, `Journal::push` records no
/// trace events. The journal itself is still appended; only the
/// observability fan-out is suppressed.
#[test]
fn journal_push_emits_nothing_when_trace_disabled() {
    trace::enable(false);
    let _ = trace::drain();
    let mut journal = Journal::new();
    journal.push(JournalEntry::SetTapped {
        iid: "X".into(),
        was: false,
        now: true,
    });
    assert!(trace::drain().is_empty());
    assert_eq!(
        journal.entries().len(),
        1,
        "the journal entry itself still records"
    );
}

/// INTENT: `state.bump_action(key, player)` emits a Count event
/// carrying the key, the player, the counter value BEFORE the
/// increment, and the value AFTER. First call: 0 → 1.
#[test]
fn bump_action_emits_count_event_zero_to_one_on_first_call() {
    fresh_trace();
    let mut state = empty_state();
    state.bump_action("draw", PlayerId::A);
    let events = trace::drain();
    let (key, player, before, after) = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Count {
                key,
                player,
                before,
                after,
                ..
            } => Some((key.clone(), *player, *before, *after)),
            _ => None,
        })
        .expect("Count event present");
    assert_eq!(key, "draw");
    assert_eq!(player, PlayerId::A);
    assert_eq!(before, 0, "first bump's `before` must be 0");
    assert_eq!(after, 1, "first bump's `after` must be 1");
}

/// INTENT: repeated bumps record the running counter accurately.
/// before[n] == after[n-1]; after[n] == before[n] + 1.
#[test]
fn repeated_bumps_track_counter_growth() {
    fresh_trace();
    let mut state = empty_state();
    for _ in 0..3 {
        state.bump_action("play", PlayerId::B);
    }
    let events = trace::drain();
    let counts: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|e| match e {
            TraceEvent::Count {
                key, before, after, ..
            } if key == "play" => Some((*before, *after)),
            _ => None,
        })
        .collect();
    assert_eq!(counts, vec![(0, 1), (1, 2), (2, 3)]);
}

/// INTENT: bump_action keys are independent. Bumping "draw" never
/// affects the "play" counter.
#[test]
fn bump_action_keys_are_independent() {
    fresh_trace();
    let mut state = empty_state();
    state.bump_action("draw", PlayerId::A);
    state.bump_action("play", PlayerId::A);
    state.bump_action("draw", PlayerId::A);
    let events = trace::drain();
    let draws: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|e| match e {
            TraceEvent::Count {
                key, before, after, ..
            } if key == "draw" => Some((*before, *after)),
            _ => None,
        })
        .collect();
    let plays: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|e| match e {
            TraceEvent::Count {
                key, before, after, ..
            } if key == "play" => Some((*before, *after)),
            _ => None,
        })
        .collect();
    assert_eq!(draws, vec![(0, 1), (1, 2)]);
    assert_eq!(plays, vec![(0, 1)]);
}

/// INTENT: bump_action counters are scoped per player. Bumping
/// for A leaves B's counter at 0.
#[test]
fn bump_action_player_scopes_are_independent() {
    fresh_trace();
    let mut state = empty_state();
    state.bump_action("draw", PlayerId::A);
    state.bump_action("draw", PlayerId::A);
    state.bump_action("draw", PlayerId::B);
    let events = trace::drain();
    let a_counts: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|e| match e {
            TraceEvent::Count {
                player,
                before,
                after,
                ..
            } if *player == PlayerId::A => Some((*before, *after)),
            _ => None,
        })
        .collect();
    let b_counts: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|e| match e {
            TraceEvent::Count {
                player,
                before,
                after,
                ..
            } if *player == PlayerId::B => Some((*before, *after)),
            _ => None,
        })
        .collect();
    assert_eq!(a_counts, vec![(0, 1), (1, 2)]);
    assert_eq!(b_counts, vec![(0, 1)]);
}

/// INTENT: when the bus is disabled, `bump_action` emits no Count
/// events. The internal counter still ticks.
#[test]
fn bump_action_emits_nothing_when_trace_disabled() {
    trace::enable(false);
    let _ = trace::drain();
    let mut state = empty_state();
    state.bump_action("draw", PlayerId::A);
    assert!(trace::drain().is_empty());
    assert_eq!(
        state.action_counts.get("draw").map(|v| v[0]).unwrap_or(0),
        1,
        "counter increments even when trace is off"
    );
}

// ----- O4: Play + Winner events --------------------------------

/// INTENT: every `play_card` call emits exactly one `Play` event,
/// even when the play fails before any mutations occur. The event
/// is the per-call summary the trace consumer reads.
#[test]
fn play_card_with_unknown_iid_emits_one_play_event_with_error_outcome() {
    fresh_trace();
    let mut state = empty_state();
    let fake_iid: InstanceId = "NOPE:0001".to_string();
    let r = state.play_card(PlayerId::A, &fake_iid, PlayChoices::default(), None);
    assert!(r.is_err(), "fake iid should fail play_card");
    let events = trace::drain();
    let plays: Vec<(InstanceId, String)> = events
        .iter()
        .filter_map(|e| match e {
            TraceEvent::Play { iid, outcome, .. } => Some((iid.clone(), outcome.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(plays.len(), 1, "expected exactly 1 Play event");
    assert_eq!(plays[0].0, fake_iid);
    assert!(
        plays[0].1.starts_with("err:"),
        "outcome should be err-prefixed, got {:?}",
        plays[0].1
    );
}

/// INTENT: when the bus is disabled, `play_card` emits no Play
/// events. Play behavior is unchanged otherwise.
#[test]
fn play_card_emits_no_event_when_trace_disabled() {
    trace::enable(false);
    let _ = trace::drain();
    let mut state = empty_state();
    let fake_iid: InstanceId = "NOPE:0001".to_string();
    let _ = state.play_card(PlayerId::A, &fake_iid, PlayChoices::default(), None);
    assert!(trace::drain().is_empty());
}

/// INTENT: `state.set_winner(Some(p), cause)` emits a Winner event
/// carrying the winning player and the cause label the caller
/// passed. The cause is the categorical reason (deckout, suicide,
/// combat_damage, lua_kill, …) — a fixed taxonomy is fine.
#[test]
fn set_winner_emits_event_with_who_and_cause() {
    fresh_trace();
    let mut state = empty_state();
    state.set_winner(Some(PlayerId::A), "test_cause");
    let events = trace::drain();
    let (who, cause) = events
        .iter()
        .find_map(|e| match e {
            TraceEvent::Winner { who, cause, .. } => Some((*who, cause.clone())),
            _ => None,
        })
        .expect("Winner event present");
    assert_eq!(who, PlayerId::A);
    assert_eq!(cause, "test_cause");
}

/// INTENT: `set_winner(None, …)` emits no Winner event. Clearing
/// the winner (rollback) is a different observation from declaring
/// one.
#[test]
fn set_winner_none_does_not_emit_winner_event() {
    fresh_trace();
    let mut state = empty_state();
    state.set_winner(None, "");
    let events = trace::drain();
    assert!(
        !events.iter().any(|e| matches!(e, TraceEvent::Winner { .. })),
        "Some(_) → Winner; None → not Winner"
    );
}

/// INTENT: when the bus is disabled, `set_winner` emits no Winner
/// events. The winner field is still set.
#[test]
fn set_winner_emits_no_event_when_trace_disabled() {
    trace::enable(false);
    let _ = trace::drain();
    let mut state = empty_state();
    state.set_winner(Some(PlayerId::B), "deckout");
    assert!(trace::drain().is_empty());
    assert_eq!(state.winner, Some(PlayerId::B));
}
