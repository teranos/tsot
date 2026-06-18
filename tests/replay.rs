//! Replay round-trip: run a game, capture the journal, save to ReplayFile,
//! serialize to JSON, deserialize, rebuild initial state, replay forward,
//! assert the result matches the original game's final state.

mod common;

use tsot::card::CardRegistry;
use tsot::choice::ScriptedOracle;
use tsot::game::{GameState, Journal};
use tsot::replay::ReplayFile;

#[test]
fn replay_round_trip_reconstructs_final_state() {
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let deck_a = common::fixed_deck(&registry, 50);
    let deck_b = common::fixed_deck(&registry, 50);
    let deck_a_ids: Vec<String> = deck_a.iter().map(|c| c.id.clone()).collect();
    let deck_b_ids: Vec<String> = deck_b.iter().map(|c| c.id.clone()).collect();

    // Original run: open replay journal, evolve game, capture journal + final.
    let mut original = GameState::new(deck_a.clone(), deck_b.clone());
    original.replay_journal = Some(Journal::new());
    let mut oracle = ScriptedOracle::new(vec![]);
    for _ in 0..3 {
        common::one_combat_cycle(&mut original, &mut oracle, registry.lua());
        if original.winner.is_some() {
            break;
        }
    }
    let original_final = format!("{:?}", debug_state_minus_replay_journal(&original));
    let original_journal = original.replay_journal.take().unwrap_or_default();

    // Build ReplayFile + serialize.
    let replay = ReplayFile {
        seed: 0,
        deck_a_card_ids: deck_a_ids,
        deck_b_card_ids: deck_b_ids,
        journal: original_journal,
    };
    let json = replay.to_json().unwrap();
    assert!(!json.is_empty());

    // Deserialize + replay forward.
    let restored: ReplayFile = ReplayFile::from_json(&json).unwrap();
    let mut replayed = restored.rebuild_initial_state(&registry).unwrap();
    restored.journal.replay_forward(&mut replayed);

    let replayed_final = format!("{:?}", debug_state_minus_replay_journal(&replayed));
    assert_eq!(
        original_final, replayed_final,
        "replay should produce byte-identical final state"
    );
}

/// Helper: snapshot state without the journals (they differ between
/// original-run-via-helpers and replay-via-forward-apply).
fn debug_state_minus_replay_journal(state: &GameState) -> String {
    // We build a debug string from individual fields to avoid journal
    // identity in the comparison.
    format!(
        "a={:?} b={:?} pool={:?} active={:?} turn={} phase={:?} winner={:?} combat={:?} fires={:?} actions={:?}",
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
    )
}
