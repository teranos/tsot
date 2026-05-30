//! Genome representation for evolutionary deck search.
//!
//! A genome is a flat `Vec<String>` of card ids — a 50-card multiset by
//! convention, though the EA loop is the only thing that enforces deck
//! size. This module owns the genome ↔ deck conversion; everything
//! downstream (fitness, selection, crossover, mutation) operates on the
//! flat string vec and only realizes the `Vec<Card>` at evaluation time.

// The matchup-runner binary doesn't call these yet — the EA loop will.
// Tests exercise them. Allow dead_code until the EA entry point lands.
#![allow(dead_code)]

use tsot::card::{Card, CardRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenomeError {
    UnknownCardId(String),
}

impl std::fmt::Display for GenomeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenomeError::UnknownCardId(id) => {
                write!(f, "genome references unknown card id {id:?}")
            }
        }
    }
}

impl std::error::Error for GenomeError {}

/// Materialize a genome into a deck the engine can play. Each id is
/// looked up in the registry and the resulting `Card` is cloned into
/// the output `Vec` in genome order. Returns `UnknownCardId` on the
/// first id that doesn't resolve — the EA loop is responsible for
/// only producing genomes drawn from a known pool.
pub fn to_deck(registry: &CardRegistry, genome: &[String]) -> Result<Vec<Card>, GenomeError> {
    let mut deck = Vec::with_capacity(genome.len());
    for id in genome {
        match registry.get(id) {
            Some(card) => deck.push(card.clone()),
            None => return Err(GenomeError::UnknownCardId(id.clone())),
        }
    }
    Ok(deck)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn load_registry() -> CardRegistry {
        CardRegistry::load(Path::new("cards")).expect("load cards/")
    }

    #[test]
    fn to_deck_round_trips_ids() {
        let registry = load_registry();
        let first_id = registry.cards()[0].id.clone();
        let genome = vec![first_id.clone(), first_id.clone(), first_id.clone()];
        let deck = to_deck(&registry, &genome).unwrap();
        assert_eq!(deck.len(), 3);
        for card in &deck {
            assert_eq!(card.id, first_id);
        }
    }

    #[test]
    fn to_deck_preserves_genome_order() {
        let registry = load_registry();
        let ids: Vec<String> = registry
            .cards()
            .iter()
            .take(5)
            .map(|c| c.id.clone())
            .collect();
        assert_eq!(ids.len(), 5);
        let deck = to_deck(&registry, &ids).unwrap();
        let deck_ids: Vec<String> = deck.iter().map(|c| c.id.clone()).collect();
        assert_eq!(deck_ids, ids);
    }

    #[test]
    fn to_deck_rejects_unknown_id() {
        let registry = load_registry();
        let bogus = "this-card-id-does-not-exist".to_string();
        let err = to_deck(&registry, std::slice::from_ref(&bogus)).unwrap_err();
        assert_eq!(err, GenomeError::UnknownCardId(bogus));
    }
}
