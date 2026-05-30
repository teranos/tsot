//! Evolution loop: random initial population → evaluate → select →
//! crossover → mutate → repair → evaluate child → carry top-K elites
//! → repeat. Pure function of [`EvolveConfig`] + registry/pool/gauntlet.
//!
//! Elites carry their cached fitness across generations (not re-
//! evaluated), so the best-of-generation trace is monotonically non-
//! decreasing while `elite_count >= 1`. This is the EA's correctness
//! contract — if it ever regresses, either elitism or fitness
//! determinism is broken.

#![allow(dead_code)]

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::{Card, CardRegistry};

use super::fitness::fitness;
use super::genome::random_genome;
use super::ops::{crossover_uniform, mutate, repair, tournament_select};

#[derive(Debug, Clone)]
pub struct EvolveConfig {
    /// Number of individuals in each generation. Constant across
    /// generations.
    pub pop_size: usize,
    /// Number of generations to run (excluding generation 0, the
    /// initial random population).
    pub generations: usize,
    /// `n_per_side` argument to `fitness`. EA.md's measured
    /// recommendation is 10.
    pub n_per_side: u32,
    /// Master seed for every random decision in the run. Same seed
    /// → byte-identical [`EvolveResult`].
    pub base_seed: u64,
    /// Cards per genome. 50.
    pub deck_len: usize,
    /// Per-card cap. 3.
    pub per_card_cap: u32,
    /// Tournament size. 3.
    pub tournament_k: usize,
    /// Per-slot mutation probability. With deck_len=50, a rate of
    /// 0.03 ≈ Poisson(1.5) mutations per child.
    pub mutation_rate: f64,
    /// Top-K individuals carry their (genome, cached fitness)
    /// unchanged to the next generation. 1 is the canonical choice.
    pub elite_count: usize,
}

impl Default for EvolveConfig {
    fn default() -> Self {
        Self {
            pop_size: 50,
            generations: 30,
            n_per_side: 10,
            base_seed: 0xEA_C8,
            deck_len: 50,
            per_card_cap: 3,
            tournament_k: 3,
            mutation_rate: 0.03,
            elite_count: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EvolveResult {
    /// Final generation, sorted by fitness descending.
    pub final_population: Vec<(Vec<String>, f64)>,
    /// Best individual of each generation, indexed by generation
    /// number (0 = initial random pop).
    pub best_per_generation: Vec<(Vec<String>, f64)>,
}

pub fn evolve(
    registry: &CardRegistry,
    pool: &[Card],
    gauntlet: &[Vec<Card>],
    cfg: &EvolveConfig,
) -> EvolveResult {
    let mut rng = StdRng::seed_from_u64(cfg.base_seed);

    let mut pop: Vec<(Vec<String>, f64)> = Vec::with_capacity(cfg.pop_size);
    for _ in 0..cfg.pop_size {
        let genome = random_genome(pool, cfg.deck_len, cfg.per_card_cap, &mut rng)
            .expect("init random_genome: pool too small for deck_len/cap");
        let fit_seed: u64 = rng.gen();
        let fit = fitness(registry, &genome, gauntlet, cfg.n_per_side, fit_seed)
            .expect("init fitness: genome contains unknown card id");
        pop.push((genome, fit));
    }

    let mut best_per_generation = Vec::with_capacity(cfg.generations + 1);
    pop.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    best_per_generation.push(pop[0].clone());

    for _gen in 0..cfg.generations {
        let mut next: Vec<(Vec<String>, f64)> = Vec::with_capacity(cfg.pop_size);

        // Elitism: top-K carry cached fitness.
        for elite in pop.iter().take(cfg.elite_count.min(pop.len())) {
            next.push(elite.clone());
        }

        // Fill remainder by select → crossover → mutate → repair → eval.
        while next.len() < cfg.pop_size {
            let parent_a = tournament_select(&pop, cfg.tournament_k, &mut rng).clone();
            let parent_b = tournament_select(&pop, cfg.tournament_k, &mut rng).clone();
            let crossed = crossover_uniform(&parent_a, &parent_b, &mut rng);
            let mut child = mutate(&crossed, pool, cfg.mutation_rate, &mut rng);
            if !repair(&mut child, pool, cfg.per_card_cap, &mut rng) {
                // Pool can't satisfy cap+len — extremely rare with the
                // current card count + cap=3. Replace with a fresh
                // random genome rather than retry the same crossover.
                child = random_genome(pool, cfg.deck_len, cfg.per_card_cap, &mut rng)
                    .expect("repair fallback random_genome: pool too small");
            }
            let fit_seed: u64 = rng.gen();
            let fit = fitness(registry, &child, gauntlet, cfg.n_per_side, fit_seed)
                .expect("child fitness: repair should have removed unknown ids");
            next.push((child, fit));
        }

        next.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        best_per_generation.push(next[0].clone());
        pop = next;
    }

    EvolveResult {
        final_population: pop,
        best_per_generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tsot::card::{CardType, CostSource};

    use super::super::fitness::{build_gauntlet, GAUNTLET_MASTER_SEED};

    fn load_registry() -> CardRegistry {
        CardRegistry::load(Path::new("cards")).unwrap()
    }

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

    /// Tiny config — keeps the test under a couple seconds.
    fn tiny_config() -> EvolveConfig {
        EvolveConfig {
            pop_size: 4,
            generations: 3,
            n_per_side: 1,
            base_seed: 0xC0DE,
            deck_len: 50,
            per_card_cap: 3,
            tournament_k: 2,
            mutation_rate: 0.05,
            elite_count: 1,
        }
    }

    #[test]
    fn evolve_returns_correct_population_size() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let cfg = tiny_config();
        let result = evolve(&reg, &pool, &gauntlet, &cfg);
        assert_eq!(result.final_population.len(), cfg.pop_size);
        // best_per_generation includes generation 0 (initial), so
        // length is generations + 1.
        assert_eq!(result.best_per_generation.len(), cfg.generations + 1);
    }

    #[test]
    fn evolve_is_deterministic_per_seed() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let cfg = tiny_config();
        let r_1 = evolve(&reg, &pool, &gauntlet, &cfg);
        let r_2 = evolve(&reg, &pool, &gauntlet, &cfg);
        let f_1: Vec<f64> = r_1.final_population.iter().map(|(_, f)| *f).collect();
        let f_2: Vec<f64> = r_2.final_population.iter().map(|(_, f)| *f).collect();
        assert_eq!(f_1, f_2, "fitness sequences diverged across identical evolve runs");
        let g_1: Vec<Vec<String>> = r_1
            .final_population
            .iter()
            .map(|(g, _)| g.clone())
            .collect();
        let g_2: Vec<Vec<String>> = r_2
            .final_population
            .iter()
            .map(|(g, _)| g.clone())
            .collect();
        assert_eq!(g_1, g_2, "final genomes diverged across identical evolve runs");
    }

    #[test]
    fn evolve_with_elitism_is_monotonic_in_best_fitness() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let cfg = tiny_config();
        let result = evolve(&reg, &pool, &gauntlet, &cfg);
        let best_fitness: Vec<f64> = result
            .best_per_generation
            .iter()
            .map(|(_, f)| *f)
            .collect();
        for w in best_fitness.windows(2) {
            assert!(
                w[1] >= w[0],
                "best fitness regressed: {} -> {} (elitism should prevent this)",
                w[0],
                w[1],
            );
        }
    }
}
