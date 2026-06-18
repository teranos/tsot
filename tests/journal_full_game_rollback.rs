//! Verifies the load-bearing invariant for journal-based search rollback
//! (MCTS, multiplayer rollback netcode, undo): if we open a journal at
//! some point in a game, drive a varied mutation sequence across every
//! engine subsystem (turn flow, play_card, declare_attacker, combat
//! resolution, handler effects), then call `Journal::rollback`, we get
//! back the exact pre-journal state — byte-identically.
//!
//! If this test fails, some mutation site is not journaled. Any code
//! that relies on rollback (sim suicide-skipping today, MCTS tomorrow)
//! would corrupt state silently. JOURNAL.md claims Sessions 1-3 covered
//! every mutation; this is the regression net.
//!
//! Distinct from `tests/replay.rs`: that test verifies FORWARD apply of
//! a serialized journal. This one verifies the INVERSE direction.

mod common;

use tsot::card::{Card, CardRegistry};
use tsot::choice::ScriptedOracle;
use tsot::game::{EventContext, GameState, Journal, Phase, PlayChoices};

/// Debug fingerprint of state, excluding the journal slots themselves
/// (since the journal is what we're testing — comparing journal-to-
/// journal after rollback would be circular). All other fields are in
/// scope. Includes `priority` (which replay.rs's fingerprint does not
/// — rollback testing cares about every engine-visible field).
fn fingerprint(state: &GameState) -> String {
    format!(
        "a={:?}|b={:?}|pool={:?}|active={:?}|turn={}|phase={:?}|winner={:?}|combat={:?}|fires={:?}|actions={:?}|priority={:?}",
        state.a,
        state.b,
        state.card_pool,
        state.active_player,
        state.turn,
        state.phase,
        state.winner,
        state.combat,
        state.event_fires,
        state.action_counts,
        state.priority,
    )
}

/// Drive a multi-turn varied mutation sequence:
///   - phase advances (untap → draw → main1 → combat → main2 → end → swap)
///   - play_card (hand → board with attached payment)
///   - declare_attacker / confirm_attacks / confirm_blocks (combat
///     mutations including damage, deaths, mill-to-exile)
///
/// 3 turns total; enough to land mutations across turn.rs, play.rs,
/// combat.rs, lua_api.rs (the four subsystems Sessions 1-3 journaled).
fn drive_scripted_game(state: &mut GameState, lua: &mlua::Lua) {
    let mut oracle = ScriptedOracle::new(vec![]);
    for _ in 0..3 {
        common::advance_to(state, Phase::Main1);
        if state.winner.is_some() {
            return;
        }

        // Try to play the first hand card with a 1-hand payment from
        // hand[1] (different card, same identity since all cards are
        // the same template).
        let active = state.active_player;
        let hand = state.player(active).hand.clone();
        if hand.len() >= 2 {
            let cast_iid = hand[0].clone();
            let payment_iid = hand[1].clone();
            let choices = PlayChoices {
                hand_payment_ids: vec![payment_iid],
                ..PlayChoices::default()
            };
            let _ = state.play_card(
                active,
                &cast_iid,
                choices,
                Some(&mut EventContext::new(lua, &mut oracle)),
            );
        }

        common::advance_to(state, Phase::Combat);
        if state.winner.is_some() {
            return;
        }

        // Summoning-sick creatures will be rejected by the engine —
        // that's fine, the mutations that DO happen still need to
        // journal correctly.
        common::declare_and_resolve_all_attackers(state, &mut oracle, lua);

        common::advance_to(state, Phase::Untap);
    }
}

#[test]
fn replay_journal_rollback_restores_full_state() {
    let registry = CardRegistry::load_embedded().expect("load embedded cards");
    let template = common::vanilla_template(&registry);
    let deck_a: Vec<Card> = (0..50).map(|_| template.clone()).collect();
    let deck_b: Vec<Card> = (0..50).map(|_| template.clone()).collect();

    let mut state = GameState::new(deck_a, deck_b);

    // Snapshot AFTER initial setup, BEFORE opening the journal.
    let snapshot = fingerprint(&state);

    // Open the replay-journal — this is the slot the engine uses for
    // game-long capture. Mutations through any subsystem write here
    // (helpers push to `journal` if open else `replay_journal`).
    state.replay_journal = Some(Journal::new());

    drive_scripted_game(&mut state, registry.lua());

    let mid = fingerprint(&state);
    assert_ne!(
        snapshot, mid,
        "the scripted sequence should have visibly mutated state"
    );

    // Take + rollback.
    let journal = state
        .replay_journal
        .take()
        .expect("replay_journal still open");
    assert!(
        !journal.is_empty(),
        "journal should have captured at least some mutations"
    );
    journal.rollback(&mut state);

    let post = fingerprint(&state);
    assert_eq!(
        snapshot, post,
        "Journal rollback failed to restore byte-identical state. \
         Some mutation site is not journaled; MCTS-style search would \
         corrupt state silently if built on the current journal. \
         Diff hint: compare the per-field portions of the two strings."
    );
}

// The strongest full-game rollback test (using `run_game_continue`)
// lives in `src/sim/run.rs`'s unit-test module because `sim` is a
// binary-only module not exposed to integration tests. See
// `tests::full_random_game_rollback_restores_initial_state` in
// `src/sim/run.rs`.

/// Determinism corollary: the same journal applied twice from the same
/// initial state should produce identical final states. Catches journal
/// entries that depend on hidden RNG state or non-deterministic ordering.
#[test]
fn replay_journal_rollback_is_idempotent_with_a_fresh_state() {
    let registry = CardRegistry::load_embedded().expect("load embedded cards");
    let template = common::vanilla_template(&registry);
    let deck_a: Vec<Card> = (0..50).map(|_| template.clone()).collect();
    let deck_b: Vec<Card> = (0..50).map(|_| template.clone()).collect();

    // Run A
    let mut state_a = GameState::new(deck_a.clone(), deck_b.clone());
    let snap_a = fingerprint(&state_a);
    state_a.replay_journal = Some(Journal::new());
    drive_scripted_game(&mut state_a, registry.lua());
    let journal_a = state_a.replay_journal.take().unwrap();
    journal_a.rollback(&mut state_a);
    let after_a = fingerprint(&state_a);

    // Run B (fresh state, same inputs)
    let mut state_b = GameState::new(deck_a, deck_b);
    let snap_b = fingerprint(&state_b);
    state_b.replay_journal = Some(Journal::new());
    drive_scripted_game(&mut state_b, registry.lua());
    let journal_b = state_b.replay_journal.take().unwrap();
    journal_b.rollback(&mut state_b);
    let after_b = fingerprint(&state_b);

    assert_eq!(snap_a, snap_b, "fresh-state setup is deterministic");
    assert_eq!(after_a, after_b, "rollback is deterministic");
    assert_eq!(snap_a, after_a, "rollback restores initial");
}
