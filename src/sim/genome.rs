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

use std::collections::BTreeMap;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;

use crate::card::{Card, CardRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenomeError {
    UnknownCardId(String),
    /// `random_genome` cannot fill `len` ids with each appearing at most
    /// `cap` times — pool doesn't have enough unique cards. Required:
    /// `pool_unique_count * cap >= len`.
    PoolTooSmall {
        len: usize,
        cap: u32,
        pool_unique: usize,
    },
}

impl std::fmt::Display for GenomeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenomeError::UnknownCardId(id) => {
                write!(f, "genome references unknown card id {id:?}")
            }
            GenomeError::PoolTooSmall {
                len,
                cap,
                pool_unique,
            } => write!(
                f,
                "cannot draw {len} ids with per-card cap {cap} from a pool of {pool_unique} unique cards (need pool_unique * cap >= len)"
            ),
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

/// Generate a random genome of `len` card ids drawn from `pool`, with no
/// id appearing more than `cap` times. Uniform random over the cards
/// that still have remaining capacity — sample → push → decrement →
/// repeat until full. Reproducible per `rng` seed.
pub fn random_genome(
    pool: &[Card],
    len: usize,
    cap: u32,
    rng: &mut StdRng,
) -> Result<Vec<String>, GenomeError> {
    let mut remaining: BTreeMap<String, u32> = BTreeMap::new();
    for c in pool {
        *remaining.entry(c.id.clone()).or_insert(0) += cap;
    }
    let pool_unique = remaining.len();
    if pool_unique * (cap as usize) < len {
        return Err(GenomeError::PoolTooSmall {
            len,
            cap,
            pool_unique,
        });
    }

    let mut genome = Vec::with_capacity(len);
    while genome.len() < len {
        // Candidates are ids still with non-zero remaining capacity.
        let candidates: Vec<&String> = remaining
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(id, _)| id)
            .collect();
        let pick = candidates.choose(rng).expect("invariant: cap sums to >= len");
        let id = (*pick).clone();
        *remaining.get_mut(&id).unwrap() -= 1;
        genome.push(id);
    }
    Ok(genome)
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

    use rand::SeedableRng;

    fn pool() -> Vec<Card> {
        load_registry().cards().to_vec()
    }

    #[test]
    fn random_genome_has_requested_length() {
        let mut rng = StdRng::seed_from_u64(0xEA);
        let g = random_genome(&pool(), 50, 3, &mut rng).unwrap();
        assert_eq!(g.len(), 50);
    }

    #[test]
    fn random_genome_respects_per_card_cap() {
        let mut rng = StdRng::seed_from_u64(0xEA);
        let g = random_genome(&pool(), 50, 3, &mut rng).unwrap();
        let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
        for id in &g {
            *counts.entry(id.as_str()).or_insert(0) += 1;
        }
        for (id, n) in &counts {
            assert!(*n <= 3, "card {id} appeared {n} times, cap is 3");
        }
    }

    #[test]
    fn random_genome_only_uses_pool_ids() {
        let p = pool();
        let pool_ids: std::collections::BTreeSet<&str> =
            p.iter().map(|c| c.id.as_str()).collect();
        let mut rng = StdRng::seed_from_u64(0xEA);
        let g = random_genome(&p, 50, 3, &mut rng).unwrap();
        for id in &g {
            assert!(
                pool_ids.contains(id.as_str()),
                "id {id} not in pool"
            );
        }
    }

    #[test]
    fn random_genome_is_deterministic_per_seed() {
        let p = pool();
        let mut rng_1 = StdRng::seed_from_u64(0xEA);
        let mut rng_2 = StdRng::seed_from_u64(0xEA);
        let g_1 = random_genome(&p, 50, 3, &mut rng_1).unwrap();
        let g_2 = random_genome(&p, 50, 3, &mut rng_2).unwrap();
        assert_eq!(g_1, g_2);
    }

    #[test]
    fn random_genome_rejects_pool_too_small() {
        // Single-card pool, cap 3, asking for 50 → cannot satisfy.
        let p = vec![pool()[0].clone()];
        let mut rng = StdRng::seed_from_u64(0xEA);
        let err = random_genome(&p, 50, 3, &mut rng).unwrap_err();
        match err {
            GenomeError::PoolTooSmall {
                len,
                cap,
                pool_unique,
            } => {
                assert_eq!(len, 50);
                assert_eq!(cap, 3);
                assert_eq!(pool_unique, 1);
            }
            other => panic!("expected PoolTooSmall, got {other:?}"),
        }
    }
}
