//! Fitness function for evolutionary deck search.
//!
//! A genome is scored against a fixed gauntlet of decks. Each opponent
//! is faced `2 × n_per_side` times — the genome plays both seats,
//! `n_per_side` games on each side, so first-mover advantage cancels.
//!
//! The gauntlet is built once from a hardcoded master seed
//! ([`GAUNTLET_MASTER_SEED`]) so its bytes are stable across runs,
//! branches, and machines. Don't change that constant after the first
//! EA run produces data — evolved-fitness numbers stop being comparable.
//!
//! Hall-of-fame extension shape (deferred): gauntlet is `Vec<Vec<Card>>`
//! not `[Vec<Card>; 7]`, so appending a champion deck every K
//! generations is a `push` away from working.

// The matchup-runner binary doesn't call these yet — the EA loop will.
#![allow(dead_code)]

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::{Card, CardRegistry};
use tsot::game::{GameState, PlayerId};

use super::deck_token::{DeckToken, Side};
use super::genome::{to_deck, GenomeError};
use super::run::run_game;
use super::variants::{build_random_deck, mandatory_for_variant, variant_pool, VARIANTS};

/// Hardcoded master seed for the EA gauntlet. Fixed forever so evolved
/// fitness numbers are comparable across days, branches, and machines.
pub const GAUNTLET_MASTER_SEED: u64 = 0xEA_C8;

/// Build the 7 variant-anchored gauntlet decks. Each variant gets one
/// canonical 50-card deck derived from `master_seed` via [`DeckToken`]'s
/// per-deck-seed mechanism, so the gauntlet bytes are reproducible.
pub fn build_gauntlet(playable_pool: &[Card], master_seed: u64) -> Vec<Vec<Card>> {
    let mut gauntlet = Vec::with_capacity(VARIANTS.len());
    for &v in &VARIANTS {
        let token = DeckToken {
            master_seed,
            side: Side::A,
            variant_a: v,
            variant_b: v,
            game_index: 0,
        };
        let pool = variant_pool(playable_pool, v);
        let mut rng = StdRng::seed_from_u64(token.per_deck_seed());
        let deck = build_random_deck(&pool, &mut rng, 50, mandatory_for_variant(v));
        gauntlet.push(deck);
    }
    gauntlet
}

/// Score a genome against the gauntlet. Plays `2 × gauntlet.len() ×
/// n_per_side` games — mirror match against each opponent. Returns
/// win-rate in `[0.0, 1.0]`.
///
/// Determinism: fitness is a pure function of `(genome, gauntlet,
/// n_per_side, base_seed)`. The internal RNG is seeded from `base_seed`
/// only; no shared external state.
pub fn fitness(
    registry: &CardRegistry,
    genome: &[String],
    gauntlet: &[Vec<Card>],
    n_per_side: u32,
    base_seed: u64,
) -> Result<f64, GenomeError> {
    let deck_g = to_deck(registry, genome)?;
    if gauntlet.is_empty() || n_per_side == 0 {
        return Ok(0.0);
    }
    let mut rng = StdRng::seed_from_u64(base_seed);
    let mut wins = 0u32;
    let mut games = 0u32;
    for opp in gauntlet {
        for _ in 0..n_per_side {
            // genome as side A
            let state = GameState::new(deck_g.clone(), opp.clone());
            let mut game_rng = StdRng::seed_from_u64(rng.gen());
            let mut log: Vec<String> = Vec::new();
            let (stats, _) = run_game(state, &mut game_rng, &mut log, registry.lua());
            if stats.winner == PlayerId::A {
                wins += 1;
            }
            games += 1;
            // genome as side B
            let state = GameState::new(opp.clone(), deck_g.clone());
            let mut game_rng = StdRng::seed_from_u64(rng.gen());
            let mut log = Vec::new();
            let (stats, _) = run_game(state, &mut game_rng, &mut log, registry.lua());
            if stats.winner == PlayerId::B {
                wins += 1;
            }
            games += 1;
        }
    }
    Ok(wins as f64 / games as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tsot::card::{CardType, CostSource};

    fn load_registry() -> CardRegistry {
        CardRegistry::load(Path::new("cards")).expect("load cards/")
    }

    // Duplicates main.rs's playable-pool filter. Pulled out only here
    // since the binary's main() owns the canonical version; this stays
    // in tests until the EA entry point lands and the filter gets
    // factored into a shared helper.
    fn playable_pool(registry: &CardRegistry) -> Vec<Card> {
        registry
            .cards()
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    CardType::Creature | CardType::Spell | CardType::Artifact | CardType::Mutation
                )
            })
            .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
            .filter(|c| {
                c.cost.iter().all(|cc| {
                    matches!(
                        cc.source,
                        CostSource::Hand
                            | CostSource::Mill
                            | CostSource::Graveyard
                            | CostSource::Sacrifice
                    )
                })
            })
            .cloned()
            .collect()
    }

    #[test]
    fn build_gauntlet_returns_one_deck_per_variant() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let g = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        assert_eq!(g.len(), VARIANTS.len());
        for deck in &g {
            assert_eq!(deck.len(), 50);
        }
    }

    #[test]
    fn build_gauntlet_is_deterministic_per_master_seed() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let g_1 = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let g_2 = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let ids_1: Vec<Vec<String>> = g_1
            .iter()
            .map(|d| d.iter().map(|c| c.id.clone()).collect())
            .collect();
        let ids_2: Vec<Vec<String>> = g_2
            .iter()
            .map(|d| d.iter().map(|c| c.id.clone()).collect())
            .collect();
        assert_eq!(ids_1, ids_2);
    }

    #[test]
    fn fitness_is_deterministic_per_seed() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        // Tiny genome built from the gauntlet's first deck — guaranteed
        // to be in the registry, no GenomeError on to_deck.
        let genome: Vec<String> = gauntlet[0].iter().map(|c| c.id.clone()).collect();
        let f_1 = fitness(&reg, &genome, &gauntlet, 1, 0xC0DE).unwrap();
        let f_2 = fitness(&reg, &genome, &gauntlet, 1, 0xC0DE).unwrap();
        assert_eq!(f_1, f_2, "fitness diverged across identical calls");
    }

    #[test]
    fn fitness_is_in_unit_interval() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0].iter().map(|c| c.id.clone()).collect();
        let f = fitness(&reg, &genome, &gauntlet, 1, 0xC0DE).unwrap();
        assert!((0.0..=1.0).contains(&f), "fitness {f} out of [0, 1]");
    }

    #[test]
    fn fitness_propagates_genome_error() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let bogus = vec!["nonexistent-card-id".to_string()];
        let err = fitness(&reg, &bogus, &gauntlet, 1, 0xC0DE).unwrap_err();
        assert_eq!(err, GenomeError::UnknownCardId("nonexistent-card-id".into()));
    }

    #[test]
    fn fitness_returns_zero_for_empty_gauntlet_or_zero_n() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0].iter().map(|c| c.id.clone()).collect();
        assert_eq!(
            fitness(&reg, &genome, &[], 1, 0xC0DE).unwrap(),
            0.0,
            "empty gauntlet should short-circuit to 0.0"
        );
        assert_eq!(
            fitness(&reg, &genome, &gauntlet, 0, 0xC0DE).unwrap(),
            0.0,
            "n=0 should short-circuit to 0.0"
        );
    }
}
