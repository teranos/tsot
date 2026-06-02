//! Replay round-trip: run a game, capture the journal, save to ReplayFile,
//! serialize to JSON, deserialize, rebuild initial state, replay forward,
//! assert the result matches the original game's final state.

use tsot::card::CardRegistry;
use tsot::choice::ScriptedOracle;
use tsot::game::{EventContext, GameState, Journal, Phase, PlayerId};
use tsot::replay::ReplayFile;

#[test]
fn replay_round_trip_reconstructs_final_state() {
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    // Use a known card as a deck filler so both runs see the same cards.
    let template = registry
        .cards()
        .iter()
        .find(|c| matches!(c.kind, tsot::card::CardType::Creature))
        .unwrap()
        .clone();
    let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
    let deck_b: Vec<_> = (0..50).map(|_| template.clone()).collect();
    let deck_a_ids: Vec<String> = deck_a.iter().map(|c| c.id.clone()).collect();
    let deck_b_ids: Vec<String> = deck_b.iter().map(|c| c.id.clone()).collect();

    // Original run: open replay journal, evolve game, capture journal + final.
    let mut original = GameState::new(deck_a.clone(), deck_b.clone());
    original.replay_journal = Some(Journal::new());
    evolve(&mut original, registry.lua());
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

fn evolve(state: &mut GameState, lua: &mlua::Lua) {
    let mut oracle = ScriptedOracle::new(vec![]);
    // Several phases including combat to exercise mutations across subsystems.
    for _ in 0..3 {
        while state.phase != Phase::Combat && state.winner.is_none() {
            state.next_phase(None);
        }
        if state.winner.is_some() {
            return;
        }
        let attackers: Vec<_> = state.player(PlayerId::A).board.to_vec();
        for atk in &attackers {
            let _ = state.declare_attacker(
                atk,
                Some(&mut EventContext::new(lua, &mut oracle)),
            );
        }
        let _ = state.confirm_attacks();
        let _ = state.confirm_blocks(Some(&mut EventContext::new(lua, &mut oracle)));
        while state.phase != Phase::Untap && state.winner.is_none() {
            state.next_phase(None);
        }
    }
}
