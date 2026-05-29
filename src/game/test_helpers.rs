//! Shared test fixtures for `game::*` test modules.
//! `#[cfg(test)]` gated; not compiled in release builds.

use super::state::{GameState, InstanceId};
use crate::card::{Card, CardType, CostComponent, Stats};

pub(crate) fn card_with_stats(id: &str, x: i32, y: i32) -> Card {
    Card {
        id: id.to_string(),
        name: String::new(),
        colors: vec![],
        kind: CardType::Creature,
        subtypes: vec![],
        symbol: String::new(),
        cost: vec![],
        abilities: vec![],
        stats: Some(Stats { x, y }),
    }
}

pub(crate) fn card_no_stats(id: &str, kind: CardType) -> Card {
    Card {
        id: id.to_string(),
        name: String::new(),
        colors: vec![],
        kind,
        subtypes: vec![],
        symbol: String::new(),
        cost: vec![],
        abilities: vec![],
        stats: None,
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
