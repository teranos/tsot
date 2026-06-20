//! Shared integration-test fixtures.
//!
//! Extracted from four test files that all needed the same "build a
//! deck, evolve a game through combat" scaffolding with subtle
//! per-test variations. Subtle variations stay as caller-supplied
//! parameters (`ScriptedOracle` contents, `Phase` targets); the
//! shared algorithmic shape lives here.
//!
//! Per Rust's integration-test idiom, this file is included in each
//! test binary that needs it via `mod common;`. It's not its own test
//! binary, so its `#[test]` functions run once per consumer binary —
//! cheap, and each consumer's helper-test failure points at the
//! specific binary that breaks.

use tsot::card::{Card, CardRegistry, CardType, CostSource};
use tsot::choice::ScriptedOracle;
use tsot::game::{EventContext, GameState, Phase};

/// Build a deck of N fresh cards from the registry's first creature
/// template, cloned. Both decks in a determinism test must be
/// byte-identical for the inputs to be considered identical; passing
/// the same `(registry, n)` produces it.
pub fn fixed_deck(registry: &CardRegistry, n: usize) -> Vec<Card> {
    let template = registry
        .cards()
        .iter()
        .find(|c| matches!(c.kind, CardType::Creature))
        .expect("registry should contain at least one creature card")
        .clone();
    (0..n).map(|_| template.clone()).collect()
}

/// A vanilla 1-hand-cost creature with no `on_*` handlers. Playable
/// without scripted oracle answers, so a test driving multi-turn
/// state mutation doesn't need to thread choice answers through.
///
/// Used by the journal-rollback test where any handler-side
/// nondeterminism would corrupt the fingerprint comparison. Returns
/// the FIRST such card the registry exposes; panics if none exists.
pub fn vanilla_template(registry: &CardRegistry) -> Card {
    registry
        .cards()
        .iter()
        .find(|c| {
            matches!(c.kind, CardType::Creature)
                && c.handlers.is_empty()
                && c.cost.len() == 1
                && c.cost[0].source == CostSource::Hand
                && c.cost[0].amount == 1
                && !c.cost[0].is_x
        })
        .expect(
            "a vanilla 1-hand creature with no handlers should exist in the embedded card corpus",
        )
        .clone()
}

/// Advance the engine cursor to the given phase. Stops early if the
/// game ends (`winner` is set) — callers check `state.winner` after
/// the call before mutating further.
pub fn advance_to(state: &mut GameState, target: Phase) {
    while state.phase != target && state.winner.is_none() {
        state.next_phase(None).expect("None ctx never yields");
    }
}

/// Declare every creature on the active player's board as an attacker,
/// then confirm the attack + confirm blocks. The 9-line sequence
/// appeared verbatim in three different test files; calling this
/// function is the dedup.
///
/// Engine refusals (summoning-sick attacker, no legal attacks, etc.)
/// are swallowed deliberately — the goal is to drive mutations
/// across as many engine subsystems as land, not to assert on
/// specific declare success.
pub fn declare_and_resolve_all_attackers(
    state: &mut GameState,
    oracle: &mut ScriptedOracle,
    lua: &mlua::Lua,
) {
    let active = state.active_player;
    let attackers: Vec<_> = state.player(active).board.to_vec();
    for atk in &attackers {
        let _ = state.declare_attacker(atk, Some(&mut EventContext::new(lua, oracle)));
    }
    let _ = state.confirm_attacks();
    let _ = state.confirm_blocks(Some(&mut EventContext::new(lua, oracle)));
}

/// One full combat cycle:
///   1. advance to `Combat`
///   2. declare every attacker + confirm + confirm blocks
///   3. advance to the next `Untap`
///
/// Caller provides a scripted oracle (empty for cards with no
/// choices, populated for tests that exercise handler choices).
/// Returns early if `state.winner` becomes set at any point.
pub fn one_combat_cycle(
    state: &mut GameState,
    oracle: &mut ScriptedOracle,
    lua: &mlua::Lua,
) {
    advance_to(state, Phase::Combat);
    if state.winner.is_some() {
        return;
    }
    declare_and_resolve_all_attackers(state, oracle, lua);
    advance_to(state, Phase::Untap);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_deck_size_matches_n() {
        let registry = CardRegistry::load(std::path::Path::new("cards"))
            .expect("cards dir at CWD");
        let deck = fixed_deck(&registry, 50);
        assert_eq!(deck.len(), 50);
    }

    #[test]
    fn fixed_deck_is_byte_identical_on_repeat_calls() {
        // Determinism gate: the same inputs must produce the same
        // deck. If this fails, fixed_deck became non-deterministic
        // and `tests/determinism.rs` would start flaking.
        let registry = CardRegistry::load(std::path::Path::new("cards"))
            .expect("cards dir at CWD");
        let d1 = fixed_deck(&registry, 50);
        let d2 = fixed_deck(&registry, 50);
        let f1: Vec<_> = d1.iter().map(|c| c.id.clone()).collect();
        let f2: Vec<_> = d2.iter().map(|c| c.id.clone()).collect();
        assert_eq!(f1, f2);
    }

    #[test]
    fn vanilla_template_has_no_handlers_and_is_1_hand() {
        let registry = CardRegistry::load(std::path::Path::new("cards"))
            .expect("cards dir at CWD");
        let v = vanilla_template(&registry);
        assert!(v.handlers.is_empty(), "vanilla means no handlers");
        assert_eq!(v.cost.len(), 1);
        assert_eq!(v.cost[0].amount, 1);
        assert_eq!(v.cost[0].source, CostSource::Hand);
        assert!(!v.cost[0].is_x);
    }

    #[test]
    fn one_combat_cycle_either_wins_or_lands_at_untap() {
        // Smoke test: a cycle either finishes the game or lands at
        // Untap (next turn). If the engine ever lands at any other
        // phase after one_combat_cycle, the contract here is broken
        // and dependent tests will become incoherent.
        let registry = CardRegistry::load(std::path::Path::new("cards"))
            .expect("cards dir at CWD");
        let deck_a = fixed_deck(&registry, 50);
        let deck_b = fixed_deck(&registry, 50);
        let mut state = GameState::new(deck_a, deck_b);
        let mut oracle = ScriptedOracle::new(vec![]);
        one_combat_cycle(&mut state, &mut oracle, registry.lua());
        assert!(
            state.winner.is_some() || state.phase == Phase::Untap,
            "after one_combat_cycle, expected winner.is_some() or phase==Untap; got winner={:?} phase={:?}",
            state.winner,
            state.phase,
        );
    }
}
