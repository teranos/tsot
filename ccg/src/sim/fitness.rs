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

use crate::card::{Card, CardRegistry};
use crate::game::{DeckUnit, GameState, PlayerId};

use super::deck_token::{DeckToken, Side};
use super::game_trace::{GameTrace, TraceSink};
use super::genome::{shuffle_units, to_units, GenomeError};
use super::run::run_game_with_ai;
use super::AiKind;
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
    /// Per-game observability: how many of the games this breakdown
    /// covers tripped at least one `[play_card-ERR]` (or related
    /// failure tag) during play. The EA was previously blind to this —
    /// a genome could win by exploiting a picker/resolver bug and the
    /// score wouldn't tell us. Drained from
    /// [`instrument::FAILURE_SINK`](crate::sim::instrument) per game.
    pub failed_games_total: u32,
}

/// Build the 7 variant-anchored gauntlet decks. Each variant gets one
/// canonical 50-card deck derived from `master_seed` via [`DeckToken`]'s
/// per-deck-seed mechanism, so the gauntlet bytes are reproducible.
pub fn build_gauntlet(playable_pool: &[Card], master_seed: u64) -> Vec<Vec<DeckUnit>> {
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
        gauntlet.push(deck.into_iter().map(DeckUnit::Card).collect());
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
    registry: &std::sync::Arc<CardRegistry>,
    genome: &[String],
    gauntlet: &[Vec<DeckUnit>],
    n_per_side: u32,
    base_seed: u64,
    ai: &AiKind,
) -> Result<f64, GenomeError> {
    fitness_breakdown(registry, genome, gauntlet, n_per_side, base_seed, ai)
        .map(|b| b.total)
}

/// Trace-emitting variant of [`fitness`]. When `trace` is `Some`, one
/// [`GameTrace`] is recorded per game — grepable post-hoc by `seed` to
/// reconstruct `(genome, opponent, seat, ai)` for that specific game.
/// Requires `opponent_ids.len() == gauntlet.len()` (one id list per opponent).
#[allow(clippy::too_many_arguments)]
pub fn fitness_with_trace(
    registry: &std::sync::Arc<CardRegistry>,
    genome: &[String],
    gauntlet: &[Vec<DeckUnit>],
    opponent_ids: &[Vec<String>],
    n_per_side: u32,
    base_seed: u64,
    ai: &AiKind,
    trace: Option<&(dyn TraceSink + 'static)>,
) -> Result<f64, GenomeError> {
    fitness_breakdown_with_trace(
        registry, genome, gauntlet, opponent_ids, n_per_side, base_seed, ai, trace,
    )
    .map(|b| b.total)
}

/// Diagnostic variant of [`fitness`] that exposes per-opponent win-rates.
/// Same byte-for-byte reproducibility as `fitness` per
/// `(genome, gauntlet, n_per_side, base_seed)`. The EA loop calls
/// `fitness` (scalar); inspection code (top-K reporting, regression
/// diffs) calls this.
pub fn fitness_breakdown(
    registry: &std::sync::Arc<CardRegistry>,
    genome: &[String],
    gauntlet: &[Vec<DeckUnit>],
    n_per_side: u32,
    base_seed: u64,
    ai: &AiKind,
) -> Result<FitnessBreakdown, GenomeError> {
    fitness_breakdown_with_trace(
        registry, genome, gauntlet, &[], n_per_side, base_seed, ai, None,
    )
}

/// Trace-emitting variant of [`fitness_breakdown`]. When `trace` is
/// `Some`, one [`GameTrace`] is recorded per game. `opponent_ids` must
/// be either empty (no trace) or the same length as `gauntlet` (one
/// id list per opponent) — a length mismatch when `trace.is_some()`
/// silently omits the trace for out-of-range opponents. The RNG
/// derivation and every gameplay call is identical to the untraced
/// path; the trace is a pure side effect.
#[allow(clippy::too_many_arguments)]
pub fn fitness_breakdown_with_trace(
    registry: &std::sync::Arc<CardRegistry>,
    genome: &[String],
    gauntlet: &[Vec<DeckUnit>],
    opponent_ids: &[Vec<String>],
    n_per_side: u32,
    base_seed: u64,
    ai: &AiKind,
    trace: Option<&(dyn TraceSink + 'static)>,
) -> Result<FitnessBreakdown, GenomeError> {
    // Build as sleeve-units so a genome carrying the cardless sentinel
    // materializes real empty sleeves. Opponent decks are ordinary cards,
    // wrapped as `DeckUnit::Card` per game below.
    let deck_g = to_units(registry.as_ref(), genome)?;
    // Discard any failure-sink entries that pre-date this evaluation —
    // they belong to whatever the worker thread ran before us. Drain
    // BEFORE the empty-gauntlet early-return so the cleanup happens
    // unconditionally; otherwise pre-existing entries would survive
    // an empty-input call and bleed into the next genome's score.
    let _ = crate::sim::instrument::drain_failures();
    if gauntlet.is_empty() || n_per_side == 0 {
        return Ok(FitnessBreakdown {
            total: 0.0,
            per_opponent: vec![0.0; gauntlet.len()],
            failed_games_total: 0,
        });
    }
    let mut rng = StdRng::seed_from_u64(base_seed);
    let mut total_wins = 0u32;
    let mut total_games = 0u32;
    let mut failed_games_total = 0u32;
    let mut per_opponent = Vec::with_capacity(gauntlet.len());
    // Both seats play the SAME `ai`. Make evolve = strongest-vs-
    // strongest by default (UCT-vs-UCT) so the fitness signal
    // measures real-deck-vs-real-deck quality, not "does the
    // candidate's deck win when the candidate is dumb." The
    // earlier asymmetric Heuristic-vs-X shape biased every score
    // toward decks that exploit Heuristic-side mistakes — fine for
    // a smoke test, misleading for evolution.
    let ais_a = [ai.clone(), ai.clone()];
    let ais_b = [ai.clone(), ai.clone()];
    for (opp_idx, opp) in gauntlet.iter().enumerate() {
        let mut opp_wins = 0u32;
        let mut opp_games = 0u32;
        let opp_ids: Option<&[String]> =
            opponent_ids.get(opp_idx).map(|v| v.as_slice());
        for _ in 0..n_per_side {
            // RULES S.0: shuffle each player's deck before the game
            // so the opening hand isn't a fixed prefix of the
            // genome. Shuffle seeds are drawn from `rng` so the
            // whole gauntlet is replayable from the outer seed.
            // Draw the two shuffle seeds AS SEEDS (not RNGs) so the
            // trace can capture and a replay can reconstruct them.
            let shuffle_seed_a: u64 = rng.gen();
            let shuffle_seed_b: u64 = rng.gen();
            // genome as side A
            let mut deck_g_a = deck_g.clone();
            let mut deck_opp_a: Vec<DeckUnit> = opp.clone();
            let mut shuf_rng_a = StdRng::seed_from_u64(shuffle_seed_a);
            let mut shuf_rng_b = StdRng::seed_from_u64(shuffle_seed_b);
            shuffle_units(&mut deck_g_a, &mut shuf_rng_a);
            shuffle_units(&mut deck_opp_a, &mut shuf_rng_b);
            let state = GameState::from_units(deck_g_a, deck_opp_a);
            // One seed drives the game AND identifies it in the timeout
            // dump — bind it once so the two can never disagree.
            let game_seed = rng.gen();
            if let (Some(sink), Some(opp_ids)) = (trace, opp_ids) {
                sink.record(GameTrace {
                    seed: game_seed,
                    shuffle_seed_a,
                    shuffle_seed_b,
                    genome,
                    opponent: opp_ids,
                    seat: 'A',
                    ai,
                });
            }
            let mut game_rng = StdRng::seed_from_u64(game_seed);
            let mut log: Vec<String> = Vec::new();
            let (stats, _) =
                run_game_with_ai(state, &mut game_rng, &mut log, registry, &ais_a, game_seed);
            if !crate::sim::instrument::drain_failures().is_empty() {
                failed_games_total += 1;
            }
            if stats.winner == PlayerId::A {
                opp_wins += 1;
            }
            opp_games += 1;
            // genome as side B
            // Seat-B mirror: seat-A is the opponent-decked player,
            // seat-B is the genome. Shuffle seed for seat A goes to
            // the OPPONENT deck, seed for seat B goes to the GENOME
            // deck — same order the RNG draws them, so the trace's
            // (shuffle_seed_a, shuffle_seed_b) field always names
            // "seat-A's shuffle seed, seat-B's shuffle seed" regardless
            // of who's playing whom.
            let shuffle_seed_a: u64 = rng.gen();
            let shuffle_seed_b: u64 = rng.gen();
            let mut deck_opp_b: Vec<DeckUnit> = opp.clone();
            let mut deck_g_b = deck_g.clone();
            let mut shuf_rng_a = StdRng::seed_from_u64(shuffle_seed_a);
            let mut shuf_rng_b = StdRng::seed_from_u64(shuffle_seed_b);
            shuffle_units(&mut deck_opp_b, &mut shuf_rng_a);
            shuffle_units(&mut deck_g_b, &mut shuf_rng_b);
            let state = GameState::from_units(deck_opp_b, deck_g_b);
            let game_seed = rng.gen();
            if let (Some(sink), Some(opp_ids)) = (trace, opp_ids) {
                sink.record(GameTrace {
                    seed: game_seed,
                    shuffle_seed_a,
                    shuffle_seed_b,
                    genome,
                    opponent: opp_ids,
                    seat: 'B',
                    ai,
                });
            }
            let mut game_rng = StdRng::seed_from_u64(game_seed);
            let mut log = Vec::new();
            let (stats, _) =
                run_game_with_ai(state, &mut game_rng, &mut log, registry, &ais_b, game_seed);
            if !crate::sim::instrument::drain_failures().is_empty() {
                failed_games_total += 1;
            }
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
        failed_games_total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use crate::card::{CardType, CostSource};

    fn load_registry() -> std::sync::Arc<CardRegistry> {
        std::sync::Arc::new(CardRegistry::load(Path::new("cards")).expect("load cards/"))
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
            .map(|d| {
                d.iter()
                    .filter_map(|u| match u {
                        DeckUnit::Card(c) => Some(c.id.clone()),
                        DeckUnit::Cardless => None,
                    })
                    .collect()
            })
            .collect();
        let ids_2: Vec<Vec<String>> = g_2
            .iter()
            .map(|d| {
                d.iter()
                    .filter_map(|u| match u {
                        DeckUnit::Card(c) => Some(c.id.clone()),
                        DeckUnit::Cardless => None,
                    })
                    .collect()
            })
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
        let genome: Vec<String> = gauntlet[0]
            .iter()
            .filter_map(|u| match u {
                DeckUnit::Card(c) => Some(c.id.clone()),
                DeckUnit::Cardless => None,
            })
            .collect();
        let f_1 = fitness(&reg, &genome, &gauntlet, 1, 0xC0DE, &AiKind::Fast).unwrap();
        let f_2 = fitness(&reg, &genome, &gauntlet, 1, 0xC0DE, &AiKind::Fast).unwrap();
        assert_eq!(f_1, f_2, "fitness diverged across identical calls");
    }

    /// Structural guard against picker/resolver disagreement. Sweeps
    /// random genomes from every archetype through the full gauntlet and
    /// asserts no game trips a `[play_card-ERR]`. `failed_games_total`
    /// counts games where the failure sink was non-empty — i.e. the sim
    /// picker handed `play_card` a choice the resolver rejected. That must
    /// be zero: a picker/resolver disagreement is always a bug, never
    /// expected game flow. (This is the guard that would have caught the
    /// crystal-tap-without-hand-cost regression at its source.)
    #[test]
    fn no_picker_resolver_disagreements_across_random_sweep() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let mut total_failed = 0u32;
        let mut total_games = 0u32;
        for v in VARIANTS {
            let vpool = variant_pool(&pool, v);
            for seed in 0..6u64 {
                let mut rng = StdRng::seed_from_u64(0x5EED_0000 + seed);
                let deck = build_random_deck(&vpool, &mut rng, 50, mandatory_for_variant(v));
                let genome: Vec<String> = deck.iter().map(|c| c.id.clone()).collect();
                let b = fitness_breakdown(&reg, &genome, &gauntlet, 1, 0xC0DE + seed, &AiKind::Fast)
                    .expect("random genome scores");
                total_failed += b.failed_games_total;
                total_games += (gauntlet.len() as u32) * 2;
            }
        }
        assert_eq!(
            total_failed, 0,
            "picker/resolver disagreement: {total_failed} of {total_games} games tripped [play_card-ERR]"
        );
    }

    #[test]
    fn fitness_is_in_unit_interval() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0]
            .iter()
            .filter_map(|u| match u {
                DeckUnit::Card(c) => Some(c.id.clone()),
                DeckUnit::Cardless => None,
            })
            .collect();
        let f = fitness(&reg, &genome, &gauntlet, 1, 0xC0DE, &AiKind::Fast).unwrap();
        assert!((0.0..=1.0).contains(&f), "fitness {f} out of [0, 1]");
    }

    #[test]
    fn fitness_propagates_genome_error() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let bogus = vec!["nonexistent-card-id".to_string()];
        let err = fitness(&reg, &bogus, &gauntlet, 1, 0xC0DE, &AiKind::Fast).unwrap_err();
        assert_eq!(err, GenomeError::UnknownCardId("nonexistent-card-id".into()));
    }

    #[test]
    fn fitness_breakdown_total_equals_mean_of_per_opponent() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0]
            .iter()
            .filter_map(|u| match u {
                DeckUnit::Card(c) => Some(c.id.clone()),
                DeckUnit::Cardless => None,
            })
            .collect();
        let b = fitness_breakdown(&reg, &genome, &gauntlet, 2, 0xC0DE, &AiKind::Fast).unwrap();
        assert_eq!(b.per_opponent.len(), gauntlet.len());
        let mean = b.per_opponent.iter().sum::<f64>() / b.per_opponent.len() as f64;
        assert!(
            (b.total - mean).abs() < 1e-12,
            "total {} != mean(per_opponent) {mean}",
            b.total,
        );
    }

    // fitness_breakdown must drain pre-existing entries from the
    // per-thread FAILURE_SINK before running its games — otherwise
    // failures from whatever the worker thread evaluated previously
    // would be miscredited to this genome. The EA worker thread
    // previously leaked these unbounded across thousands of fitness
    // evaluations.
    //
    // Uses empty gauntlet so we exercise the drain logic without
    // running any games (which would convolve real engine failures
    // with the cleanup we're testing). The drain runs BEFORE the
    // empty-gauntlet early-return for exactly this reason.
    #[test]
    fn fitness_breakdown_drains_pre_existing_failure_sink_entries() {
        let reg = load_registry();
        crate::sim::instrument::push_failure("pre-existing entry".to_string());
        crate::sim::instrument::push_failure("another pre-existing entry".to_string());
        let breakdown =
            fitness_breakdown(&reg, &[], &[], 0, 0xC0DE, &AiKind::Fast).unwrap();
        let leftover = crate::sim::instrument::drain_failures();
        assert!(
            leftover.is_empty(),
            "fitness_breakdown left failure-sink entries behind: {leftover:?}"
        );
        assert_eq!(
            breakdown.failed_games_total, 0,
            "pre-existing entries must never be credited to this run"
        );
    }

    // failed_games_total is bounded by the number of games actually
    // run. A heuristic-vs-heuristic Ra match with n_per_side=1 plays
    // exactly 2 × gauntlet.len() games — failed_games_total can never
    // exceed that.
    #[test]
    fn fitness_breakdown_failed_games_total_bounded_by_total_games() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0]
            .iter()
            .filter_map(|u| match u {
                DeckUnit::Card(c) => Some(c.id.clone()),
                DeckUnit::Cardless => None,
            })
            .collect();
        let breakdown =
            fitness_breakdown(&reg, &genome, &gauntlet, 1, 0xC0DE, &AiKind::Fast).unwrap();
        let total_games = (gauntlet.len() as u32) * 2; // 2 per opponent at n_per_side=1
        assert!(
            breakdown.failed_games_total <= total_games,
            "failed_games_total {} > total_games {total_games}",
            breakdown.failed_games_total,
        );
        // And after the call the sink is drained (no leak across calls).
        let leftover = crate::sim::instrument::drain_failures();
        assert!(leftover.is_empty(), "post-call sink not empty: {leftover:?}");
    }

    #[test]
    fn fitness_matches_breakdown_total() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0]
            .iter()
            .filter_map(|u| match u {
                DeckUnit::Card(c) => Some(c.id.clone()),
                DeckUnit::Cardless => None,
            })
            .collect();
        let scalar = fitness(&reg, &genome, &gauntlet, 2, 0xC0DE, &AiKind::Fast).unwrap();
        let breakdown = fitness_breakdown(&reg, &genome, &gauntlet, 2, 0xC0DE, &AiKind::Fast).unwrap();
        assert_eq!(scalar, breakdown.total);
    }

    /// `fitness_with_trace` must emit one [`GameTrace`] per game with
    /// the same `seed` that `run_game_with_ai` was invoked with,
    /// covering both seats. Without this guarantee the trace can't
    /// serve its reproducibility purpose (a missing or drifting seed
    /// makes the JSONL row unfindable when grepping heartbeat output).
    #[test]
    fn fitness_with_trace_emits_one_record_per_game_with_matching_seed() {
        use crate::sim::game_trace::{GameTrace, TraceSink};
        use std::sync::Mutex;

        struct Collector {
            records: Mutex<Vec<(u64, char, String, usize, usize)>>,
        }
        impl TraceSink for Collector {
            fn record(&self, t: GameTrace<'_>) {
                let ai_dbg = format!("{:?}", t.ai);
                self.records
                    .lock()
                    .unwrap()
                    .push((t.seed, t.seat, ai_dbg, t.genome.len(), t.opponent.len()));
            }
        }

        // Helper: extract card ids from a DeckUnit vec, skipping any
        // cardless slots (the built-in gauntlet has none, but be safe).
        fn ids_of(units: &[DeckUnit]) -> Vec<String> {
            units
                .iter()
                .filter_map(|u| match u {
                    DeckUnit::Card(c) => Some(c.id.clone()),
                    DeckUnit::Cardless => None,
                })
                .collect()
        }

        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        // Trim to one opponent for a small test: n_per_side=1 →
        // exactly 2 games (seat A + seat B) → exactly 2 trace records.
        let gauntlet = vec![gauntlet[0].clone()];
        let opp_ids: Vec<Vec<String>> = gauntlet.iter().map(|d| ids_of(d)).collect();
        let genome: Vec<String> = ids_of(&gauntlet[0]);

        let sink = Collector { records: Mutex::new(Vec::new()) };
        let _ = fitness_with_trace(
            &reg, &genome, &gauntlet, &opp_ids, 1, 0xC0DE, &AiKind::Fast, Some(&sink),
        )
        .unwrap();

        let records = sink.records.lock().unwrap();
        assert_eq!(records.len(), 2, "one record per game (2 games at n=1)");
        // Seats must be A then B (mirror-match iteration order).
        assert_eq!(records[0].1, 'A', "first record: seat A");
        assert_eq!(records[1].1, 'B', "second record: seat B");
        // ai kind propagates as debug-formatted AiKind.
        assert_eq!(records[0].2, "Fast");
        assert_eq!(records[1].2, "Fast");
        // Genome + opponent id-list sizes match what fitness saw.
        assert_eq!(records[0].3, genome.len(), "genome length in trace");
        assert_eq!(records[0].4, opp_ids[0].len(), "opponent length in trace");
        // Distinct seeds — mirror games draw two `rng.gen()`s.
        assert_ne!(records[0].0, records[1].0, "seat A and B seeds differ");
    }

    #[test]
    fn fitness_returns_zero_for_empty_gauntlet_or_zero_n() {
        let reg = load_registry();
        let pool = playable_pool(&reg);
        let gauntlet = build_gauntlet(&pool, GAUNTLET_MASTER_SEED);
        let genome: Vec<String> = gauntlet[0]
            .iter()
            .filter_map(|u| match u {
                DeckUnit::Card(c) => Some(c.id.clone()),
                DeckUnit::Cardless => None,
            })
            .collect();
        assert_eq!(
            fitness(&reg, &genome, &[], 1, 0xC0DE, &AiKind::Fast).unwrap(),
            0.0,
            "empty gauntlet should short-circuit to 0.0"
        );
        assert_eq!(
            fitness(&reg, &genome, &gauntlet, 0, 0xC0DE, &AiKind::Fast).unwrap(),
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
                .map(|s| fitness(&reg, &baseline, &gauntlet, n, 0xD00D + s, &AiKind::Fast).unwrap())
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
                .map(|g| fitness(&reg, g, &gauntlet, n, 0xD00D, &AiKind::Fast).unwrap())
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
                .map(|s| fitness(&reg, &baseline, &gauntlet, n, 0xD00D + s, &AiKind::Fast).unwrap())
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
