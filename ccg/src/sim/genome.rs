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

/// RULES S.0: shuffle a deck uniformly at random using the
/// provided RNG. Mutates in place via Fisher-Yates (`SliceRandom::shuffle`).
/// Callers building a `GameState` for a real (non-test) game MUST call
/// this on each player's deck after `to_deck` and before `GameState::new`,
/// so the opening 5-card hand isn't deterministic from genome order.
pub fn shuffle_deck(deck: &mut [Card], rng: &mut StdRng) {
    deck.shuffle(rng);
}

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

/// Like [`to_deck`] but produces [`DeckUnit`](crate::game::DeckUnit)s so
/// a decklist can carry cardless sleeves: the
/// [`CARDLESS_SLEEVE_ID`](crate::replay::CARDLESS_SLEEVE_ID) sentinel
/// becomes `DeckUnit::Cardless`, and every other id is looked up and
/// wrapped as `DeckUnit::Card`. This is the build path for decks that
/// mix real cards and empty sleeves — the starter presets that ship an
/// empty sleeve. Returns `UnknownCardId` on the first id (other than the
/// sentinel) that doesn't resolve.
pub fn to_units(
    registry: &CardRegistry,
    ids: &[String],
) -> Result<Vec<crate::game::DeckUnit>, GenomeError> {
    use crate::game::DeckUnit;
    let mut units = Vec::with_capacity(ids.len());
    for id in ids {
        if id == crate::replay::CARDLESS_SLEEVE_ID {
            units.push(DeckUnit::Cardless);
        } else {
            match registry.get(id) {
                Some(card) => units.push(DeckUnit::Card(card.clone())),
                None => return Err(GenomeError::UnknownCardId(id.clone())),
            }
        }
    }
    Ok(units)
}

/// Shuffle a `DeckUnit` deck in place — the `DeckUnit` twin of
/// [`shuffle_deck`], for decks that carry cardless sleeves.
pub fn shuffle_units(units: &mut [crate::game::DeckUnit], rng: &mut StdRng) {
    units.shuffle(rng);
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

    /// INTENT: `shuffle_deck` preserves the multiset — same cards,
    /// same counts. Permutation only, no insertion / deletion.
    #[test]
    fn shuffle_deck_preserves_multiset() {
        use rand::SeedableRng;
        let registry = load_registry();
        let ids: Vec<String> = registry
            .cards()
            .iter()
            .take(50)
            .map(|c| c.id.clone())
            .collect();
        let mut deck = to_deck(&registry, &ids).unwrap();
        let before: BTreeMap<String, u32> = deck
            .iter()
            .map(|c| c.id.clone())
            .fold(BTreeMap::new(), |mut m, id| {
                *m.entry(id).or_insert(0) += 1;
                m
            });
        let mut rng = StdRng::seed_from_u64(0xC0DE_C0DE);
        shuffle_deck(&mut deck, &mut rng);
        let after: BTreeMap<String, u32> = deck
            .iter()
            .map(|c| c.id.clone())
            .fold(BTreeMap::new(), |mut m, id| {
                *m.entry(id).or_insert(0) += 1;
                m
            });
        assert_eq!(before, after, "shuffle must not lose or duplicate cards");
        assert_eq!(deck.len(), 50);
    }

    /// INTENT: `shuffle_deck` is deterministic per seed.
    /// Same seed + same input = same output permutation.
    /// Critical for replay / reproducibility.
    #[test]
    fn shuffle_deck_is_deterministic_per_seed() {
        use rand::SeedableRng;
        let registry = load_registry();
        let ids: Vec<String> = registry
            .cards()
            .iter()
            .take(50)
            .map(|c| c.id.clone())
            .collect();
        let mut deck1 = to_deck(&registry, &ids).unwrap();
        let mut deck2 = to_deck(&registry, &ids).unwrap();
        let mut rng1 = StdRng::seed_from_u64(0xC0DE_C0DE);
        let mut rng2 = StdRng::seed_from_u64(0xC0DE_C0DE);
        shuffle_deck(&mut deck1, &mut rng1);
        shuffle_deck(&mut deck2, &mut rng2);
        let order1: Vec<String> = deck1.iter().map(|c| c.id.clone()).collect();
        let order2: Vec<String> = deck2.iter().map(|c| c.id.clone()).collect();
        assert_eq!(order1, order2);
    }

    /// INTENT: different seeds produce different orderings (almost
    /// always). A 50-card shuffle has 50! ≈ 3×10^64 permutations;
    /// the probability two distinct seeds yield the same order is
    /// vanishingly small.
    #[test]
    fn shuffle_deck_different_seeds_yield_different_order() {
        use rand::SeedableRng;
        let registry = load_registry();
        let ids: Vec<String> = registry
            .cards()
            .iter()
            .take(50)
            .map(|c| c.id.clone())
            .collect();
        let mut deck1 = to_deck(&registry, &ids).unwrap();
        let mut deck2 = to_deck(&registry, &ids).unwrap();
        let mut rng1 = StdRng::seed_from_u64(1);
        let mut rng2 = StdRng::seed_from_u64(2);
        shuffle_deck(&mut deck1, &mut rng1);
        shuffle_deck(&mut deck2, &mut rng2);
        let order1: Vec<String> = deck1.iter().map(|c| c.id.clone()).collect();
        let order2: Vec<String> = deck2.iter().map(|c| c.id.clone()).collect();
        assert_ne!(
            order1, order2,
            "two distinct seeds should produce different shuffles"
        );
    }

    use rand::SeedableRng;

    // CardRegistry owns a `mlua::Lua` VM. `Card.handlers` contains
    // `mlua::Function` values that reference that VM. If the
    // registry is dropped while Card clones are still alive, any
    // subsequent operation that touches a handler's `ValueRef`
    // (including `Card::clone()` cloning the inner mlua::Function
    // and bumping its registry ref-count) panics with "Lua instance
    // is destroyed" inside mlua's value_ref.rs.
    //
    // The pre-fix `pool()` was `load_registry().cards().to_vec()` —
    // the registry was a temporary, dropped before the Vec<Card>
    // escaped the function. Tests that only BORROWED the Vec worked;
    // tests that EXTRACTED + CLONED a Card (e.g. `pool()[0].clone()`)
    // hit the dead VM and panicked.
    //
    // Fix: keep the registry alive for the test thread's entire run
    // via a thread-local OnceCell that leaks the registry on first
    // call. Memory leak is fine in tests. CardRegistry is `!Send`
    // (the Lua VM), so a thread_local is the right shape — tests
    // run on multiple threads in parallel and each gets its own
    // long-lived registry.
    fn long_lived_registry() -> &'static CardRegistry {
        use std::cell::OnceCell;
        thread_local! {
            static THREAD_REGISTRY: OnceCell<&'static CardRegistry> =
                const { OnceCell::new() };
        }
        THREAD_REGISTRY.with(|c| {
            *c.get_or_init(|| {
                Box::leak(Box::new(
                    CardRegistry::load(Path::new("cards")).expect("load cards/"),
                ))
            })
        })
    }

    fn pool() -> Vec<Card> {
        long_lived_registry().cards().to_vec()
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
