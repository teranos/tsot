//! Shared test fixtures for `game::*` test modules.
//! `#[cfg(test)]` gated; not compiled in release builds.

use super::state::{GameState, InstanceId};
use crate::card::{Card, CardType, CostComponent, Stats};
use std::collections::BTreeMap;

pub(crate) fn card_with_stats(id: &str, x: i32, y: i32) -> Card {
    Card {
        id: id.to_string(),
        name: String::new(),
        colors: vec![],
        kind: CardType::Creature,
        timing: None,
        subtypes: vec![],
        cannot_block_subtypes: vec![],
        can_block_subtypes: vec![],
        symbols: Vec::new(),
        cost: vec![],
        abilities: vec![],
        flavor: String::new(),
        stats: Some(Stats { x, y }),
        static_def: None,
        handlers: BTreeMap::new(),
        activated: vec![],
        target: None,
        gy_hand_substitute: false,
        allow_x_zero: false,
        is_variant: false,
        variant_of: None,
    }
}

pub(crate) fn card_no_stats(id: &str, kind: CardType) -> Card {
    let timing = if kind == CardType::Spell {
        Some(crate::card::Timing::Instant)
    } else {
        None
    };
    Card {
        id: id.to_string(),
        name: String::new(),
        colors: vec![],
        kind,
        timing,
        subtypes: vec![],
        cannot_block_subtypes: vec![],
        can_block_subtypes: vec![],
        symbols: Vec::new(),
        cost: vec![],
        abilities: vec![],
        flavor: String::new(),
        stats: None,
        static_def: None,
        handlers: BTreeMap::new(),
        activated: vec![],
        target: None,
        gy_hand_substitute: false,
        allow_x_zero: false,
        is_variant: false,
        variant_of: None,
    }
}

pub(crate) fn deck_of(n: usize, prefix: &str) -> Vec<Card> {
    (0..n)
        .map(|i| card_with_stats(&format!("{prefix}-{i}"), 1, 1))
        .collect()
}

pub(crate) fn set_cost(state: &mut GameState, iid: &InstanceId, cost: Vec<CostComponent>) {
    state.card_pool.get_mut(iid).unwrap().card.cost = cost;
}

/// Mutate a card's identity (colors + symbols) in-place. Used by
/// tests for HAND-cost identity-match rules. The `symbol` parameter
/// stays single-string for ergonomic test wiring; pass "" for no
/// symbols, pass "X" for a one-element symbols Vec.
pub(crate) fn set_identity(
    state: &mut GameState,
    iid: &InstanceId,
    colors: &[&str],
    symbol: &str,
) {
    let entry = state.card_pool.get_mut(iid).unwrap();
    entry.card.colors = colors.iter().map(|c| c.to_string()).collect();
    entry.card.symbols = if symbol.is_empty() {
        Vec::new()
    } else {
        vec![symbol.to_string()]
    };
}
