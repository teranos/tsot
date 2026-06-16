//! EA operators: selection, crossover, mutation, repair.
//!
//! All functions take an explicit `&mut StdRng` and produce reproducible
//! output per seed. None of them touch the engine — they operate purely
//! on `Vec<String>` genomes and the pool of valid card ids.
//!
//! Repair is the lone gatekeeper of the per-card cap invariant. Crossover
//! and mutation can produce over-cap genomes; the caller is responsible
//! for running `repair` before passing the genome to `fitness`.

#![allow(dead_code)]

use std::collections::BTreeMap;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

use crate::card::Card;

/// Tournament selection. Sample `k` indices into `scores` uniformly
/// with replacement and return the index of the highest-scoring one.
/// Lower `k` → less selection pressure; `k=3` is the standard
/// starting point.
///
/// `scores` is decoupled from any specific population shape: pass raw
/// fitness for a vanilla EA, or `sim::diversity::selection_scores` for
/// a Jaccard-penalty-shaped selection. The RNG consumption (`k` calls
/// to `gen_range`) is independent of the score values, so swapping
/// score vectors keeps downstream RNG sequences byte-identical.
pub fn tournament_select(
    scores: &[f64],
    k: usize,
    rng: &mut StdRng,
) -> usize {
    assert!(!scores.is_empty(), "tournament_select on empty population");
    assert!(k > 0, "tournament k must be > 0");
    let mut best_idx = rng.gen_range(0..scores.len());
    let mut best_score = scores[best_idx];
    for _ in 1..k {
        let cand_idx = rng.gen_range(0..scores.len());
        let cand_score = scores[cand_idx];
        if cand_score > best_score {
            best_idx = cand_idx;
            best_score = cand_score;
        }
    }
    best_idx
}

/// Uniform crossover: for each slot, independently pick from parent_a
/// or parent_b with probability 0.5. Length is preserved (output =
/// parent_a.len() = parent_b.len()). The output may violate the
/// per-card cap — caller must `repair` before evaluation.
pub fn crossover_uniform(
    parent_a: &[String],
    parent_b: &[String],
    rng: &mut StdRng,
) -> Vec<String> {
    assert_eq!(
        parent_a.len(),
        parent_b.len(),
        "crossover parents must have equal length"
    );
    let mut child = Vec::with_capacity(parent_a.len());
    for i in 0..parent_a.len() {
        if rng.gen_bool(0.5) {
            child.push(parent_a[i].clone());
        } else {
            child.push(parent_b[i].clone());
        }
    }
    child
}

/// Mutate by per-slot random replacement. Each slot is independently
/// replaced by a random pool card with probability `rate`. With
/// `rate = λ/len` this gives Poisson(λ)-distributed mutations in
/// expectation. Output may violate the per-card cap.
pub fn mutate(
    genome: &[String],
    pool: &[Card],
    rate: f64,
    rng: &mut StdRng,
) -> Vec<String> {
    assert!((0.0..=1.0).contains(&rate), "mutation rate {rate} out of [0, 1]");
    assert!(!pool.is_empty(), "mutate from empty pool");
    let mut out = genome.to_vec();
    for slot in out.iter_mut() {
        if rng.gen_bool(rate) {
            let new = pool.choose(rng).unwrap();
            *slot = new.id.clone();
        }
    }
    out
}

/// Repair a genome so every id appears at most `cap` times. Walks
/// left-to-right, replacing over-cap occurrences with a random pool
/// card that still has remaining capacity. Length preserved.
/// Returns `false` if the pool is too small to satisfy the cap
/// constraint (genome unchanged in that case).
pub fn repair(
    genome: &mut [String],
    pool: &[Card],
    cap: u32,
    rng: &mut StdRng,
) -> bool {
    let pool_unique: std::collections::BTreeSet<&str> =
        pool.iter().map(|c| c.id.as_str()).collect();
    if pool_unique.len() * (cap as usize) < genome.len() {
        return false;
    }

    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for id in genome.iter() {
        *counts.entry(id.clone()).or_insert(0) += 1;
    }

    for slot in genome.iter_mut() {
        let id = slot.clone();
        // Pool membership: if a slot's id isn't in the pool at all,
        // treat it as over-cap and replace. Otherwise check count.
        let in_pool = pool_unique.contains(id.as_str());
        if !in_pool || *counts.get(&id).unwrap_or(&0) > cap {
            let candidates: Vec<&Card> = pool
                .iter()
                .filter(|c| *counts.get(&c.id).unwrap_or(&0) < cap)
                // dedup by id — same Card may appear multiple times in pool
                .scan(std::collections::BTreeSet::<String>::new(), |seen, c| {
                    if seen.insert(c.id.clone()) {
                        Some(Some(c))
                    } else {
                        Some(None)
                    }
                })
                .flatten()
                .collect();
            if candidates.is_empty() {
                return false;
            }
            let pick = candidates.choose(rng).unwrap();
            let new_id = pick.id.clone();
            *counts.entry(id.clone()).or_insert(0) -= 1;
            *counts.entry(new_id.clone()).or_insert(0) += 1;
            *slot = new_id;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::path::Path;
    use crate::card::CardRegistry;

    // See sim/genome.rs's identical helper for the full why. Short
    // version: CardRegistry owns the Lua VM. Cards hold mlua::Function
    // handlers that reference that VM. A pre-fix `pool()` that
    // returned `to_vec()` from a temporary registry left the cards'
    // handler refs dangling; any subsequent `Card::clone()` panicked
    // with "Lua instance is destroyed". Thread-local OnceCell that
    // leaks the registry on first call keeps it alive for the test
    // thread's lifetime — memory leak is fine in tests, CardRegistry
    // is !Send so cross-thread sharing is wrong anyway.
    fn long_lived_registry() -> &'static CardRegistry {
        use std::cell::OnceCell;
        thread_local! {
            static THREAD_REGISTRY: OnceCell<&'static CardRegistry> =
                const { OnceCell::new() };
        }
        THREAD_REGISTRY.with(|c| {
            *c.get_or_init(|| {
                Box::leak(Box::new(
                    CardRegistry::load(Path::new("cards")).unwrap(),
                ))
            })
        })
    }

    fn pool() -> Vec<Card> {
        long_lived_registry().cards().to_vec()
    }

    fn dummy_genome(len: usize, id_pattern: impl Fn(usize) -> String) -> Vec<String> {
        (0..len).map(id_pattern).collect()
    }

    #[test]
    fn tournament_select_picks_fittest_at_high_k() {
        // With k = scores.len(), tournament becomes a full argmax over the scores.
        let scores = vec![0.1, 0.9, 0.5];
        let mut rng = StdRng::seed_from_u64(0xEA);
        for _ in 0..50 {
            let winner = tournament_select(&scores, scores.len() * 10, &mut rng);
            assert_eq!(winner, 1, "max-score index should always win at high k");
        }
    }

    #[test]
    fn tournament_select_is_deterministic_per_seed() {
        let scores: Vec<f64> = (0..20).map(|i| (i as f64) / 20.0).collect();
        let mut rng_1 = StdRng::seed_from_u64(0xEA);
        let mut rng_2 = StdRng::seed_from_u64(0xEA);
        let w1 = tournament_select(&scores, 3, &mut rng_1);
        let w2 = tournament_select(&scores, 3, &mut rng_2);
        assert_eq!(w1, w2);
    }

    #[test]
    fn crossover_uniform_preserves_length() {
        let a = dummy_genome(50, |i| format!("a{i}"));
        let b = dummy_genome(50, |i| format!("b{i}"));
        let mut rng = StdRng::seed_from_u64(0xEA);
        let child = crossover_uniform(&a, &b, &mut rng);
        assert_eq!(child.len(), 50);
    }

    #[test]
    fn crossover_uniform_only_uses_parent_ids() {
        let a = dummy_genome(50, |i| format!("a{i}"));
        let b = dummy_genome(50, |i| format!("b{i}"));
        let mut rng = StdRng::seed_from_u64(0xEA);
        let child = crossover_uniform(&a, &b, &mut rng);
        for (i, id) in child.iter().enumerate() {
            assert!(
                id == &a[i] || id == &b[i],
                "slot {i} got {id} but parents had {} / {}",
                a[i],
                b[i]
            );
        }
    }

    #[test]
    fn crossover_uniform_is_deterministic_per_seed() {
        let a = dummy_genome(50, |i| format!("a{i}"));
        let b = dummy_genome(50, |i| format!("b{i}"));
        let c1 = crossover_uniform(&a, &b, &mut StdRng::seed_from_u64(0xEA));
        let c2 = crossover_uniform(&a, &b, &mut StdRng::seed_from_u64(0xEA));
        assert_eq!(c1, c2);
    }

    #[test]
    fn mutate_zero_rate_is_identity() {
        let p = pool();
        let g = dummy_genome(50, |_| p[0].id.clone());
        let mut rng = StdRng::seed_from_u64(0xEA);
        let m = mutate(&g, &p, 0.0, &mut rng);
        assert_eq!(m, g);
    }

    #[test]
    fn mutate_full_rate_replaces_every_slot_with_pool_ids() {
        let p = pool();
        let g = dummy_genome(50, |_| "definitely-not-in-pool".into());
        let mut rng = StdRng::seed_from_u64(0xEA);
        let m = mutate(&g, &p, 1.0, &mut rng);
        let pool_ids: std::collections::BTreeSet<&str> =
            p.iter().map(|c| c.id.as_str()).collect();
        for id in &m {
            assert!(
                pool_ids.contains(id.as_str()),
                "mutated id {id} should be in pool"
            );
        }
    }

    #[test]
    fn mutate_is_deterministic_per_seed() {
        let p = pool();
        let g = dummy_genome(50, |_| p[0].id.clone());
        let m1 = mutate(&g, &p, 0.1, &mut StdRng::seed_from_u64(0xEA));
        let m2 = mutate(&g, &p, 0.1, &mut StdRng::seed_from_u64(0xEA));
        assert_eq!(m1, m2);
    }

    #[test]
    fn repair_enforces_cap() {
        let p = pool();
        // Hand-craft a genome with 50 copies of a single card.
        let mut g = dummy_genome(50, |_| p[0].id.clone());
        let mut rng = StdRng::seed_from_u64(0xEA);
        assert!(repair(&mut g, &p, 3, &mut rng));
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for id in &g {
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
        for (id, n) in &counts {
            assert!(*n <= 3, "card {id} occurred {n} times after repair");
        }
        assert_eq!(g.len(), 50);
    }

    #[test]
    fn repair_replaces_unknown_ids() {
        let p = pool();
        let mut g = vec!["unknown-1".into(), "unknown-2".into(), p[0].id.clone()];
        let mut rng = StdRng::seed_from_u64(0xEA);
        assert!(repair(&mut g, &p, 3, &mut rng));
        let pool_ids: std::collections::BTreeSet<&str> =
            p.iter().map(|c| c.id.as_str()).collect();
        for id in &g {
            assert!(
                pool_ids.contains(id.as_str()),
                "id {id} not in pool after repair"
            );
        }
    }

    #[test]
    fn repair_fails_when_pool_too_small() {
        // Pool of 1 unique card, cap 3, genome of length 50 → impossible.
        let p = pool();
        let single = vec![p[0].clone()];
        let mut g = dummy_genome(50, |_| p[0].id.clone());
        let mut rng = StdRng::seed_from_u64(0xEA);
        assert!(!repair(&mut g, &single, 3, &mut rng));
    }

    #[test]
    fn repair_is_deterministic_per_seed() {
        let p = pool();
        let mut g1 = dummy_genome(50, |_| p[0].id.clone());
        let mut g2 = dummy_genome(50, |_| p[0].id.clone());
        let mut rng1 = StdRng::seed_from_u64(0xEA);
        let mut rng2 = StdRng::seed_from_u64(0xEA);
        assert!(repair(&mut g1, &p, 3, &mut rng1));
        assert!(repair(&mut g2, &p, 3, &mut rng2));
        assert_eq!(g1, g2);
    }
}
