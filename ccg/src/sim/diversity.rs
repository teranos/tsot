//! Diversity primitives used by selection (EA loop) and by the post-hoc
//! champion clustering CLIs. The single shared `jaccard` implementation
//! lives here so the three call sites can't drift.
//!
//! Selection-time policy: subtract `alpha · mean_jaccard_to_others(i)`
//! from each individual's raw fitness before tournament. `alpha = 0.0`
//! is a fast path that returns the raw fitnesses unchanged — runs with
//! the flag default off are byte-identical to pre-diversity-aware runs.

#![allow(dead_code)]

use std::collections::BTreeSet;

/// Jaccard similarity in `[0.0, 1.0]` on two id sets (multiplicity
/// collapsed). Two empty sets are defined as similarity 1.0 — matches
/// the pre-existing call sites' behavior and keeps clustering well-
/// defined for degenerate genomes.
pub fn jaccard<T: Ord>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

/// For each genome in `pop`, mean Jaccard similarity to every *other*
/// genome (self excluded). With `pop.len() <= 1` the result is filled
/// with zeros — no others to compare to, no diversity penalty owed.
pub fn mean_jaccard_to_others(pop: &[(Vec<String>, f64)]) -> Vec<f64> {
    let n = pop.len();
    if n <= 1 {
        return vec![0.0; n];
    }
    let sets: Vec<BTreeSet<String>> = pop
        .iter()
        .map(|(g, _)| g.iter().cloned().collect())
        .collect();
    let denom = (n - 1) as f64;
    let mut out = vec![0.0_f64; n];
    for i in 0..n {
        let mut sum = 0.0;
        for j in 0..n {
            if i == j {
                continue;
            }
            sum += jaccard(&sets[i], &sets[j]);
        }
        out[i] = sum / denom;
    }
    out
}

/// Selection-score vector used by tournament selection:
/// `score[i] = fitness[i] - alpha · mean_jaccard_to_others[i]`.
///
/// `alpha == 0.0` is a fast path that returns raw fitnesses without
/// computing Jaccard — this is the byte-identical-to-old-behavior gate.
pub fn selection_scores(pop: &[(Vec<String>, f64)], alpha: f64) -> Vec<f64> {
    if alpha == 0.0 {
        return pop.iter().map(|(_, f)| *f).collect();
    }
    let mean_d = mean_jaccard_to_others(pop);
    pop.iter()
        .zip(mean_d.iter())
        .map(|((_, f), m)| f - alpha * m)
        .collect()
}

/// Mean pairwise Jaccard *distance* (`1 - similarity`) across the
/// population. Used by tests/diagnostics to summarize "how spread out
/// is this population." Returns 0.0 for populations of size < 2.
pub fn mean_pairwise_distance(pop: &[(Vec<String>, f64)]) -> f64 {
    let n = pop.len();
    if n < 2 {
        return 0.0;
    }
    let sets: Vec<BTreeSet<String>> = pop
        .iter()
        .map(|(g, _)| g.iter().cloned().collect())
        .collect();
    let mut sum = 0.0_f64;
    let mut pairs = 0u32;
    for i in 0..n {
        for j in (i + 1)..n {
            sum += 1.0 - jaccard(&sets[i], &sets[j]);
            pairs += 1;
        }
    }
    sum / pairs as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn jaccard_disjoint_is_zero() {
        assert_eq!(jaccard(&s(&["a", "b"]), &s(&["c", "d"])), 0.0);
    }

    #[test]
    fn jaccard_identical_is_one() {
        assert_eq!(jaccard(&s(&["a", "b", "c"]), &s(&["a", "b", "c"])), 1.0);
    }

    #[test]
    fn jaccard_partial_overlap() {
        // {a,b,c} ∩ {b,c,d} = {b,c}  (size 2)
        // {a,b,c} ∪ {b,c,d} = {a,b,c,d}  (size 4)
        assert!((jaccard(&s(&["a", "b", "c"]), &s(&["b", "c", "d"])) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn jaccard_both_empty_is_one() {
        assert_eq!(jaccard::<String>(&BTreeSet::new(), &BTreeSet::new()), 1.0);
    }

    #[test]
    fn jaccard_one_empty_is_zero() {
        assert_eq!(jaccard(&s(&["a"]), &BTreeSet::new()), 0.0);
    }

    #[test]
    fn selection_scores_alpha_zero_returns_raw_fitness() {
        let pop = vec![
            (vec!["a".to_string()], 0.5),
            (vec!["b".to_string()], 0.8),
            (vec!["c".to_string()], 0.1),
        ];
        assert_eq!(selection_scores(&pop, 0.0), vec![0.5, 0.8, 0.1]);
    }

    #[test]
    fn selection_scores_penalize_similar_individuals() {
        // Two clones at the top, one outlier. With alpha=1.0:
        //   mean_jaccard[clone_a] = (jacc(a,b) + jacc(a,c)) / 2 = (1.0 + 0.0)/2 = 0.5
        //   mean_jaccard[clone_b] = 0.5
        //   mean_jaccard[outlier] = (0.0 + 0.0)/2 = 0.0
        // → scores = [0.5 - 0.5, 0.5 - 0.5, 0.5 - 0.0] = [0.0, 0.0, 0.5]
        let pop = vec![
            (vec!["a".to_string(), "b".to_string()], 0.5),
            (vec!["a".to_string(), "b".to_string()], 0.5),
            (vec!["c".to_string(), "d".to_string()], 0.5),
        ];
        let scores = selection_scores(&pop, 1.0);
        assert!((scores[0] - 0.0).abs() < 1e-12, "clone got {}", scores[0]);
        assert!((scores[1] - 0.0).abs() < 1e-12, "clone got {}", scores[1]);
        assert!((scores[2] - 0.5).abs() < 1e-12, "outlier got {}", scores[2]);
    }

    #[test]
    fn selection_scores_alpha_preserves_ranking_within_niche() {
        // Two clones with different raw fitness: the higher one still
        // wins after penalty (penalty depends on similarity, not fitness).
        let pop = vec![
            (vec!["a".to_string()], 0.8),
            (vec!["a".to_string()], 0.5),
            (vec!["b".to_string()], 0.5),
        ];
        let scores = selection_scores(&pop, 0.3);
        assert!(scores[0] > scores[1], "higher raw fitness should still rank higher within a niche");
    }

    #[test]
    fn mean_jaccard_to_others_empty_pop() {
        let pop: Vec<(Vec<String>, f64)> = vec![];
        assert!(mean_jaccard_to_others(&pop).is_empty());
    }

    #[test]
    fn mean_jaccard_to_others_single() {
        let pop = vec![(vec!["a".to_string()], 0.5)];
        assert_eq!(mean_jaccard_to_others(&pop), vec![0.0]);
    }

    #[test]
    fn mean_pairwise_distance_identical_pop_is_zero() {
        let pop = vec![
            (vec!["a".to_string(), "b".to_string()], 0.5),
            (vec!["a".to_string(), "b".to_string()], 0.5),
        ];
        assert_eq!(mean_pairwise_distance(&pop), 0.0);
    }

    #[test]
    fn mean_pairwise_distance_disjoint_pop_is_one() {
        let pop = vec![
            (vec!["a".to_string()], 0.5),
            (vec!["b".to_string()], 0.5),
        ];
        assert_eq!(mean_pairwise_distance(&pop), 1.0);
    }
}
