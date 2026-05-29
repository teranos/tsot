//! Determinism test: identical inputs produce byte-identical engine state.
//!
//! The clippy.toml + Cargo.toml lints prevent the obvious non-determinism
//! sources (HashMap iteration order, thread_rng). This test catches anything
//! the lints miss — if a future change introduces non-determinism somewhere
//! exotic, this fails.

use tsot::card::CardRegistry;
use tsot::choice::{ScriptedAnswer, ScriptedOracle};
use tsot::game::{EventContext, GameState, PlayerId, Phase};

/// Build a deck of N fresh cards using the registry's first creature
/// repeatedly, so both games see the same input deck.
fn fixed_deck(registry: &CardRegistry, n: usize) -> Vec<tsot::card::Card> {
    let template = registry
        .cards()
        .iter()
        .find(|c| matches!(c.kind, tsot::card::CardType::Creature))
        .unwrap()
        .clone();
    (0..n).map(|_| template.clone()).collect()
}

fn evolve_game(state: &mut GameState, lua: &mlua::Lua) {
    let mut oracle = ScriptedOracle::new(vec![
        ScriptedAnswer::Card(None),
        ScriptedAnswer::Confirm(false),
        ScriptedAnswer::Card(None),
        ScriptedAnswer::Confirm(false),
    ]);
    // Advance to Combat.
    while state.phase != Phase::Combat && state.winner.is_none() {
        state.next_phase();
    }
    if state.winner.is_some() {
        return;
    }
    // Try to declare every eligible attacker.
    let attackers: Vec<_> = state.player(PlayerId::A).board.to_vec();
    for atk in &attackers {
        let _ = state.declare_attacker(
            atk,
            Some(&mut EventContext::new(lua, &mut oracle)),
        );
    }
    let _ = state.confirm_attacks();
    let _ = state.confirm_blocks(Some(&mut EventContext::new(lua, &mut oracle)));
    // Run to end of turn.
    while state.phase != Phase::Untap && state.winner.is_none() {
        state.next_phase();
    }
}

#[test]
fn two_runs_with_identical_inputs_produce_identical_state() {
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let deck_a = fixed_deck(&registry, 50);
    let deck_b = fixed_deck(&registry, 50);

    let mut s1 = GameState::new(deck_a.clone(), deck_b.clone());
    let mut s2 = GameState::new(deck_a, deck_b);

    evolve_game(&mut s1, registry.lua());
    evolve_game(&mut s2, registry.lua());

    // Debug formatting is the cheapest canonical representation we have.
    // If anything diverges (HashMap iteration, modifier order, etc.) this
    // shows the diff immediately.
    let d1 = format!("{:?}", s1);
    let d2 = format!("{:?}", s2);
    assert_eq!(d1, d2, "two runs with identical inputs diverged");
}
