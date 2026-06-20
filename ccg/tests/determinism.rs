//! Determinism test: identical inputs produce byte-identical engine state.
//!
//! The clippy.toml + Cargo.toml lints prevent the obvious non-determinism
//! sources (HashMap iteration order, thread_rng). This test catches anything
//! the lints miss — if a future change introduces non-determinism somewhere
//! exotic, this fails.

mod common;

use tsot::card::CardRegistry;
use tsot::choice::{ScriptedAnswer, ScriptedOracle};
use tsot::game::GameState;

#[test]
fn two_runs_with_identical_inputs_produce_identical_state() {
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let deck_a = common::fixed_deck(&registry, 50);
    let deck_b = common::fixed_deck(&registry, 50);

    let mut s1 = GameState::new(deck_a.clone(), deck_b.clone());
    let mut s2 = GameState::new(deck_a, deck_b);

    let mut oracle1 = ScriptedOracle::new(vec![
        ScriptedAnswer::Card(None),
        ScriptedAnswer::Confirm(false),
        ScriptedAnswer::Card(None),
        ScriptedAnswer::Confirm(false),
    ]);
    let mut oracle2 = ScriptedOracle::new(vec![
        ScriptedAnswer::Card(None),
        ScriptedAnswer::Confirm(false),
        ScriptedAnswer::Card(None),
        ScriptedAnswer::Confirm(false),
    ]);
    common::one_combat_cycle(&mut s1, &mut oracle1, registry.lua());
    common::one_combat_cycle(&mut s2, &mut oracle2, registry.lua());

    // Debug formatting is the cheapest canonical representation we have.
    // If anything diverges (HashMap iteration, modifier order, etc.) this
    // shows the diff immediately.
    let d1 = format!("{:?}", s1);
    let d2 = format!("{:?}", s2);
    assert_eq!(d1, d2, "two runs with identical inputs diverged");
}
