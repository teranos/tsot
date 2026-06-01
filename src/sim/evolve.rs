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
    /// If `Some(k)`, terminate the run as soon as the best-of-
    /// generation has been exactly 1.0 (ceiling) for `k` consecutive
    /// generations — the fitness metric is bounded at 1.0 and can't
    /// distinguish further improvement at the current `n_per_side`.
    /// `k >= 2` recommended because n=10's stddev=0.043 means a single
    /// 1.000 measurement is a noisy observation, not proof.
    pub stop_at_ceiling: Option<usize>,
    /// `balance-probe` support: force every genome to contain at least
    /// `pinned_count` copies of `pinned_card_id`. Initial population is
    /// seeded with the pin, and after mutate+repair the pin is re-
    /// enforced. Lets the EA optimize the rest of the deck around a
    /// fixed candidate card so the resulting fitness is a measure of
    /// "what's the best deck I can build with this card forced in."
    /// None = no pin (regular evolve).
    pub pinned_card_id: Option<String>,
    /// Number of copies of `pinned_card_id` to enforce in every
    /// genome. Bounded by `per_card_cap`. Ignored when `pinned_card_id`
    /// is None.
    pub pinned_count: usize,
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
            stop_at_ceiling: None,
            pinned_card_id: None,
            pinned_count: 0,
        }
    }
}

/// Re-enforce the pin invariant on `genome`: if `pinned_card_id` is set
/// and the genome has fewer than `pinned_count` copies, replace random
/// non-pinned slots with the pinned id until the count is satisfied.
/// No-op when there's no pin. Caller must ensure `pinned_count` is
/// within `per_card_cap`.
fn enforce_pin(
    genome: &mut [String],
    pinned_card_id: Option<&str>,
    pinned_count: usize,
    rng: &mut StdRng,
) {
    let Some(pid) = pinned_card_id else { return };
    if pinned_count == 0 {
        return;
    }
    let current = genome.iter().filter(|s| s.as_str() == pid).count();
    if current >= pinned_count {
        return;
    }
    let mut deficit = pinned_count - current;
    // Indices of slots NOT already holding the pinned card. Shuffled so
    // we don't always clobber the same positions across generations.
    let mut candidate_indices: Vec<usize> = (0..genome.len())
        .filter(|i| genome[*i] != pid)
        .collect();
    use rand::seq::SliceRandom;
    candidate_indices.shuffle(rng);
    for idx in candidate_indices {
        if deficit == 0 {
            break;
        }
        genome[idx] = pid.to_string();
        deficit -= 1;
    }
}

/// True if the last `k` entries of `best_history` all hit the fitness
/// ceiling (1.0 within f64 epsilon). Used by [`evolve`] to terminate
/// runs that have plateaued at the metric's upper bound.
pub fn should_stop_at_ceiling(best_history: &[(Vec<String>, f64)], k: usize) -> bool {
    if k == 0 || best_history.len() < k {
        return false;
    }
    best_history
        .iter()
        .rev()
        .take(k)
        .all(|(_, f)| *f >= 1.0 - f64::EPSILON)
}

/// Callback fired after each generation is fully scored and sorted.
/// Receives the generation index (0 = initial random population) and
/// the current population, sorted by fitness descending.
pub type GenerationCallback<'a> = dyn FnMut(usize, &[(Vec<String>, f64)]) + 'a;

#[derive(Debug, Clone)]
pub struct EvolveResult {
    /// Final generation, sorted by fitness descending.
    pub final_population: Vec<(Vec<String>, f64)>,
    /// Best individual of each generation, indexed by generation
    /// number (0 = initial random pop).
    pub best_per_generation: Vec<(Vec<String>, f64)>,
    /// Per-generation card presence: for each generation, how many
    /// population members contain at least one copy of each card id.
    /// Indexed: `per_gen_card_freq[gen_idx][card_id] = present_count`.
    /// Used to produce the card-frequency-over-time heatmap.
    pub per_gen_card_freq: Vec<std::collections::BTreeMap<String, u32>>,
    /// Mean fitness of each generation (sum of all individuals / pop).
    pub per_gen_mean_fitness: Vec<f64>,
}

pub fn evolve(
    registry: &CardRegistry,
    pool: &[Card],
    gauntlet: &[Vec<Card>],
    cfg: &EvolveConfig,
    on_generation: &mut GenerationCallback<'_>,
) -> EvolveResult {
    let mut rng = StdRng::seed_from_u64(cfg.base_seed);

    // Wire format for the gauntlet across worker threads: each deck as
    // a Vec<String> of card ids. Workers materialize their own
    // `Vec<Card>` from their own thread-local registry.
    let gauntlet_ids = crate::sim::parallel_eval::gauntlet_to_ids(gauntlet);

    // Phase 1 (sequential, deterministic): generate the initial random
    // population genomes + their fit_seeds. RNG ordering is preserved.
    // When a pin is configured, the genome gets `pinned_count` copies
    // of the pinned id forced in (replacing random slots) before
    // fitness scoring.
    let pin_id: Option<&str> = cfg.pinned_card_id.as_deref();
    let init_jobs: Vec<(Vec<String>, u64)> = (0..cfg.pop_size)
        .map(|_| {
            let mut genome = random_genome(pool, cfg.deck_len, cfg.per_card_cap, &mut rng)
                .expect("init random_genome: pool too small for deck_len/cap");
            enforce_pin(&mut genome, pin_id, cfg.pinned_count, &mut rng);
            let fit_seed: u64 = rng.gen();
            (genome, fit_seed)
        })
        .collect();
    // Phase 2 (parallel): fan fitness scoring across worker threads.
    // Pure function of (genome, gauntlet, fit_seed) → deterministic
    // results regardless of which thread scored each.
    let init_fits = crate::sim::parallel_eval::parallel_evaluate_genomes(
        &gauntlet_ids,
        &init_jobs,
        cfg.n_per_side,
    );
    let mut pop: Vec<(Vec<String>, f64)> = init_jobs
        .into_iter()
        .zip(init_fits)
        .map(|((g, _), f)| (g, f))
        .collect();
    // The single-threaded `fitness()` (used during gauntlet sanity
    // checks below) is kept available via the unused `registry` arg.
    let _ = registry;

    let mut best_per_generation = Vec::with_capacity(cfg.generations + 1);
    let mut per_gen_card_freq: Vec<std::collections::BTreeMap<String, u32>> =
        Vec::with_capacity(cfg.generations + 1);
    let mut per_gen_mean_fitness: Vec<f64> = Vec::with_capacity(cfg.generations + 1);
    pop.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    best_per_generation.push(pop[0].clone());
    per_gen_card_freq.push(card_freq_in_pop(&pop));
    per_gen_mean_fitness.push(pop.iter().map(|(_, f)| *f).sum::<f64>() / pop.len() as f64);
    on_generation(0, &pop);

    for gen_idx in 0..cfg.generations {
        let mut next: Vec<(Vec<String>, f64)> = Vec::with_capacity(cfg.pop_size);

        // Elitism: top-K carry cached fitness.
        for elite in pop.iter().take(cfg.elite_count.min(pop.len())) {
            next.push(elite.clone());
        }

        // Phase 1 (sequential, deterministic): generate children +
        // fit_seeds. RNG ordering preserved.
        let mut child_jobs: Vec<(Vec<String>, u64)> = Vec::with_capacity(cfg.pop_size - next.len());
        while child_jobs.len() + next.len() < cfg.pop_size {
            let parent_a = tournament_select(&pop, cfg.tournament_k, &mut rng).clone();
            let parent_b = tournament_select(&pop, cfg.tournament_k, &mut rng).clone();
            let crossed = crossover_uniform(&parent_a, &parent_b, &mut rng);
            let mut child = mutate(&crossed, pool, cfg.mutation_rate, &mut rng);
            if !repair(&mut child, pool, cfg.per_card_cap, &mut rng) {
                child = random_genome(pool, cfg.deck_len, cfg.per_card_cap, &mut rng)
                    .expect("repair fallback random_genome: pool too small");
            }
            // Pin re-enforcement after mutate+repair. Mutation may have
            // replaced pinned slots with random cards; repair doesn't
            // know about the pin. This step restores the invariant
            // every child satisfies before fitness scoring.
            enforce_pin(&mut child, pin_id, cfg.pinned_count, &mut rng);
            let fit_seed: u64 = rng.gen();
            child_jobs.push((child, fit_seed));
        }
        // Phase 2 (parallel): batch-evaluate child fitnesses.
        let child_fits = crate::sim::parallel_eval::parallel_evaluate_genomes(
            &gauntlet_ids,
            &child_jobs,
            cfg.n_per_side,
        );
        next.extend(
            child_jobs
                .into_iter()
                .zip(child_fits)
                .map(|((g, _), f)| (g, f)),
        );

        next.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        best_per_generation.push(next[0].clone());
        per_gen_card_freq.push(card_freq_in_pop(&next));
        per_gen_mean_fitness.push(next.iter().map(|(_, f)| *f).sum::<f64>() / next.len() as f64);
        on_generation(gen_idx + 1, &next);
        pop = next;

        if let Some(k) = cfg.stop_at_ceiling {
            if should_stop_at_ceiling(&best_per_generation, k) {
                break;
            }
        }
    }

    EvolveResult {
        final_population: pop,
        best_per_generation,
        per_gen_card_freq,
        per_gen_mean_fitness,
    }
}

/// Count, for each card id present in any genome in the population,
/// how many population members contain at least one copy.
fn card_freq_in_pop(pop: &[(Vec<String>, f64)]) -> std::collections::BTreeMap<String, u32> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    for (genome, _) in pop {
        let unique: BTreeSet<&str> = genome.iter().map(|s| s.as_str()).collect();
        for id in unique {
            *out.entry(id.to_string()).or_insert(0) += 1;
        }
    }
    out
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
            stop_at_ceiling: None,
            pinned_card_id: None,
            pinned_count: 0,
        }
    }

    #[test]
    fn evolve_returns_correct_population_size() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let cfg = tiny_config();
        let result = evolve(&reg, &pool, &gauntlet, &cfg, &mut |_, _| {});
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
        let r_1 = evolve(&reg, &pool, &gauntlet, &cfg, &mut |_, _| {});
        let r_2 = evolve(&reg, &pool, &gauntlet, &cfg, &mut |_, _| {});
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
    fn should_stop_at_ceiling_requires_k_consecutive_ones() {
        let g = |f: f64| (vec!["x".to_string()], f);
        assert!(!should_stop_at_ceiling(&[], 3), "empty history should not stop");
        assert!(
            !should_stop_at_ceiling(&[g(1.0), g(1.0)], 3),
            "too few entries should not stop"
        );
        assert!(
            should_stop_at_ceiling(&[g(1.0), g(1.0), g(1.0)], 3),
            "exact k consecutive 1.0s should stop"
        );
        assert!(
            should_stop_at_ceiling(&[g(0.5), g(1.0), g(1.0), g(1.0)], 3),
            "last k consecutive 1.0s should stop"
        );
        assert!(
            !should_stop_at_ceiling(&[g(1.0), g(0.5), g(1.0), g(1.0)], 3),
            "a non-1.0 in the last k should not stop"
        );
        assert!(
            !should_stop_at_ceiling(&[g(0.999), g(0.999), g(0.999)], 3),
            "just below 1.0 should not stop"
        );
        assert!(
            !should_stop_at_ceiling(&[g(1.0), g(1.0)], 0),
            "k=0 should never stop"
        );
    }

    #[test]
    fn pinned_card_is_present_in_every_genome_every_generation() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        // Pick a card guaranteed to be in the pool.
        let pin_id = pool[0].id.clone();
        let cfg = EvolveConfig {
            pinned_card_id: Some(pin_id.clone()),
            pinned_count: 2,
            ..tiny_config()
        };
        let result = evolve(&reg, &pool, &gauntlet, &cfg, &mut |_, _| {});
        // Every member of the final population must contain >= 2 copies.
        for (genome, _) in &result.final_population {
            let count = genome.iter().filter(|s| **s == pin_id).count();
            assert!(
                count >= 2,
                "pin invariant broken: genome has {count} copies of {pin_id}, expected >= 2"
            );
        }
    }

    #[test]
    fn evolve_with_elitism_is_monotonic_in_best_fitness() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let cfg = tiny_config();
        let result = evolve(&reg, &pool, &gauntlet, &cfg, &mut |_, _| {});
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
