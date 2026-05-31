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

/// Per-opponent breakdown of a genome's fitness. `total` is the aggregate
/// win-rate over all opponents (what `fitness` returns as a scalar);
/// `per_opponent[i]` is the win-rate against `gauntlet[i]`, indexed in
/// the same order [`build_gauntlet`] produces (matches the [`VARIANTS`]
/// order). Always: `total == mean(per_opponent)`.
#[derive(Debug, Clone, PartialEq)]
pub struct FitnessBreakdown {
    pub total: f64,
    pub per_opponent: Vec<f64>,
}

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
    fitness_breakdown(registry, genome, gauntlet, n_per_side, base_seed).map(|b| b.total)
}

/// Diagnostic variant of [`fitness`] that exposes per-opponent win-rates.
/// Same byte-for-byte reproducibility as `fitness` per
/// `(genome, gauntlet, n_per_side, base_seed)`. The EA loop calls
/// `fitness` (scalar); inspection code (top-K reporting, regression
/// diffs) calls this.
pub fn fitness_breakdown(
    registry: &CardRegistry,
    genome: &[String],
    gauntlet: &[Vec<Card>],
    n_per_side: u32,
    base_seed: u64,
) -> Result<FitnessBreakdown, GenomeError> {
    let deck_g = to_deck(registry, genome)?;
    if gauntlet.is_empty() || n_per_side == 0 {
        return Ok(FitnessBreakdown {
            total: 0.0,
            per_opponent: vec![0.0; gauntlet.len()],
        });
    }
    let mut rng = StdRng::seed_from_u64(base_seed);
    let mut total_wins = 0u32;
    let mut total_games = 0u32;
    let mut per_opponent = Vec::with_capacity(gauntlet.len());
    for opp in gauntlet {
        let mut opp_wins = 0u32;
        let mut opp_games = 0u32;
        for _ in 0..n_per_side {
            // genome as side A
            let state = GameState::new(deck_g.clone(), opp.clone());
            let mut game_rng = StdRng::seed_from_u64(rng.gen());
            let mut log: Vec<String> = Vec::new();
            let (stats, _) = run_game(state, &mut game_rng, &mut log, registry.lua());
            if stats.winner == PlayerId::A {
                opp_wins += 1;
            }
            opp_games += 1;
            // genome as side B
            let state = GameState::new(opp.clone(), deck_g.clone());
            let mut game_rng = StdRng::seed_from_u64(rng.gen());
            let mut log = Vec::new();
            let (stats, _) = run_game(state, &mut game_rng, &mut log, registry.lua());
            if stats.winner == PlayerId::B {
                opp_wins += 1;
            }
            opp_games += 1;
        }
        per_opponent.push(opp_wins as f64 / opp_games as f64);
        total_wins += opp_wins;
        total_games += opp_games;
    }
    Ok(FitnessBreakdown {
        total: total_wins as f64 / total_games as f64,
        per_opponent,
    })
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
    fn fitness_breakdown_total_equals_mean_of_per_opponent() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0].iter().map(|c| c.id.clone()).collect();
        let b = fitness_breakdown(&reg, &genome, &gauntlet, 2, 0xC0DE).unwrap();
        assert_eq!(b.per_opponent.len(), gauntlet.len());
        let mean = b.per_opponent.iter().sum::<f64>() / b.per_opponent.len() as f64;
        assert!(
            (b.total - mean).abs() < 1e-12,
            "total {} != mean(per_opponent) {mean}",
            b.total,
        );
    }

    #[test]
    fn fitness_matches_breakdown_total() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0].iter().map(|c| c.id.clone()).collect();
        let scalar = fitness(&reg, &genome, &gauntlet, 2, 0xC0DE).unwrap();
        let breakdown = fitness_breakdown(&reg, &genome, &gauntlet, 2, 0xC0DE).unwrap();
        assert_eq!(scalar, breakdown.total);
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

    // ---------------------------------------------------------------
    // Variance measurement — load-bearing for the EA design.
    //
    // The EA only produces signal if `between-genome stddev` (the
    // spread of fitness across different decks) exceeds `within-genome
    // stddev` (the noise from re-evaluating the same deck with
    // different base_seeds). If within > between, generation-to-
    // generation improvement is indistinguishable from RNG.
    //
    // Run with:
    //   cargo test --release --bin tsot measure_fitness_variance \
    //              -- --ignored --nocapture
    //
    // Numbers go into EA.md once measured.
    // ---------------------------------------------------------------

    use super::super::genome::random_genome;

    fn mean_stddev(xs: &[f64]) -> (f64, f64) {
        let n = xs.len() as f64;
        let mean = xs.iter().sum::<f64>() / n;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        (mean, var.sqrt())
    }

    #[test]
    #[ignore]
    fn measure_fitness_variance() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);

        // Build a single baseline genome for within-genome variance,
        // and 10 random genomes for between-genome spread.
        let mut g_rng = StdRng::seed_from_u64(0xBA5E);
        let baseline = random_genome(&pool, 50, 3, &mut g_rng).unwrap();
        let genomes: Vec<Vec<String>> = (0..10)
            .map(|i| {
                let mut rng = StdRng::seed_from_u64(0xC0FFEE + i);
                random_genome(&pool, 50, 3, &mut rng).unwrap()
            })
            .collect();

        let n_values = [3u32, 5, 10, 20];
        let k_seeds = 10;

        println!();
        println!("=== Within-genome variance (1 baseline, {k_seeds} base_seeds) ===");
        println!(
            "{:>4}  {:>6}  {:>10}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}",
            "n", "games", "wall/eval", "mean", "stddev", "cv", "min", "max"
        );
        for &n in &n_values {
            let t0 = std::time::Instant::now();
            let xs: Vec<f64> = (0..k_seeds)
                .map(|s| fitness(&reg, &baseline, &gauntlet, n, 0xD00D + s).unwrap())
                .collect();
            let elapsed = t0.elapsed();
            let (mean, stddev) = mean_stddev(&xs);
            let cv = if mean > 0.0 { stddev / mean } else { 0.0 };
            let min = xs.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let per_eval = elapsed / (k_seeds as u32);
            let games = 2 * gauntlet.len() as u32 * n;
            println!(
                "{n:>4}  {games:>6}  {per_eval:>10.0?}  {mean:>6.3}  {stddev:>6.3}  {cv:>6.3}  {min:>6.3}  {max:>6.3}"
            );
        }

        println!();
        println!(
            "=== Between-genome spread ({} random genomes, 1 base_seed) ===",
            genomes.len()
        );
        println!(
            "{:>4}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}",
            "n", "games", "mean", "stddev", "min", "max"
        );
        let mut between_stddev_by_n: Vec<(u32, f64)> = Vec::new();
        for &n in &n_values {
            let xs: Vec<f64> = genomes
                .iter()
                .map(|g| fitness(&reg, g, &gauntlet, n, 0xD00D).unwrap())
                .collect();
            let (mean, stddev) = mean_stddev(&xs);
            let min = xs.iter().cloned().fold(f64::INFINITY, f64::min);
            let max = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let games = 2 * gauntlet.len() as u32 * n;
            println!(
                "{n:>4}  {games:>6}  {mean:>6.3}  {stddev:>6.3}  {min:>6.3}  {max:>6.3}"
            );
            between_stddev_by_n.push((n, stddev));
        }

        println!();
        println!("=== Signal-to-noise (between_stddev / within_stddev) ===");
        println!("{:>4}  {:>8}  {:>8}  {:>6}", "n", "within", "between", "SNR");
        for &n in &n_values {
            let within_xs: Vec<f64> = (0..k_seeds)
                .map(|s| fitness(&reg, &baseline, &gauntlet, n, 0xD00D + s).unwrap())
                .collect();
            let (_, within_sd) = mean_stddev(&within_xs);
            let between_sd = between_stddev_by_n
                .iter()
                .find(|(nn, _)| *nn == n)
                .map(|(_, sd)| *sd)
                .unwrap();
            let snr = if within_sd > 0.0 {
                between_sd / within_sd
            } else {
                f64::INFINITY
            };
            println!(
                "{n:>4}  {within_sd:>8.3}  {between_sd:>8.3}  {snr:>6.2}"
            );
        }
        println!();
        println!(
            "Interpretation: SNR > 1 means the EA can discriminate decks. SNR >= 2 is comfortable signal."
        );
    }
}
