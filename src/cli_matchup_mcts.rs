//! `tsot matchup-mcts` subcommand: pit MCTS-driven Pattern B picks
//! against the existing heuristic AI in a head-to-head matchup.
//! Measures whether one-ply rollout search beats the random-weighted
//! heuristic picker in a mirror-match shape.
//!
//! Per pairing run: N games with MCTS on side A vs Heuristic on side B,
//! then N games with sides swapped. Aggregate wins → "MCTS win rate
//! across both sides." 50% means MCTS gives no signal above heuristic.
//! 55-65% is the "infrastructure works and search helps" zone. Above
//! 70% suggests the heuristic has obvious gaps the search exposes.
//!
//! Deck shape for v1: both sides use the SAME random-genome deck per
//! game (mirror match). Removes deck-quality as a confounder; isolates
//! the AI's contribution. Future versions can take `--deck PATH` to
//! pin specific decks.

use clap::Parser;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::{Card, CardRegistry};
use tsot::game::{GameState, PlayerId};

use crate::parse_u64_hex_or_dec;
use crate::sim::evolved_deck::EvolvedDeck;
use crate::sim::genome::{random_genome, to_deck};
use crate::sim::mcts::{
    self, MctsConfig, MCTS_PICK_CALLS, MCTS_SEARCHED_PICKS, MCTS_TOTAL_CANDIDATES,
};
use crate::sim::run::run_game_continue;
use crate::sim::AiKind;
use std::sync::atomic::Ordering;

#[derive(Parser)]
pub struct MatchupMctsArgs {
    /// Games per side (total = 2 × this). Half played with MCTS on
    /// side A, half with MCTS on side B.
    #[arg(long, default_value_t = 10)]
    pub games: u32,
    /// MCTS rollouts per candidate per pick. Higher = stronger pick,
    /// linearly slower. Default 5 (~50× heuristic game time).
    #[arg(long = "rollouts", default_value_t = 5)]
    pub rollouts_per_candidate: u32,
    /// MCTS max candidates to search. Caps hand size's branching.
    #[arg(long = "max-candidates", default_value_t = 10)]
    pub max_candidates: u32,
    /// Master seed. Each game derives its own from (master, idx).
    #[arg(long, default_value_t = 0xBEEF_FACE, value_parser = parse_u64_hex_or_dec)]
    pub seed: u64,
    /// MCTS base seed. Separated from `--seed` so the same deck
    /// pairing can be re-tested with different MCTS RNG.
    #[arg(long = "mcts-seed", default_value_t = 0xC0_FFEE, value_parser = parse_u64_hex_or_dec)]
    pub mcts_seed: u64,
    /// Path to an `EvolvedDeck` JSON to use as the mirror-match deck.
    /// When set, both sides play this exact deck — eliminates deck-
    /// quality as a confounder + ensures the deck has actually-
    /// playable cards every turn (random-genome decks are usually
    /// too sparse and Pattern B short-circuits to 1 candidate,
    /// making MCTS a no-op). Defaults to `baselines/<first>.json`
    /// if unset. Ignored when both `--deck-a` and `--deck-b` are set.
    #[arg(long = "deck", value_name = "PATH")]
    pub deck: Option<String>,
    /// Deck for player A. When set together with `--deck-b`, runs
    /// asymmetric games (different deck per side) instead of a
    /// mirror match. The MCTS-on-A round plays deck-a as A and
    /// deck-b as B; the MCTS-on-B round swaps decks too (deck-b on
    /// A, deck-a on B) so MCTS plays deck-a in BOTH rounds — the
    /// win-rate measures "MCTS+deck-a vs Heuristic+deck-b."
    #[arg(long = "deck-a", value_name = "PATH")]
    pub deck_a: Option<String>,
    /// Deck for player B. See `--deck-a`.
    #[arg(long = "deck-b", value_name = "PATH")]
    pub deck_b: Option<String>,
    /// Handicap MCTS by forcing it onto the lower-saved-fitness deck.
    /// Only effective with auto-pick (no explicit `--deck` /
    /// `--deck-a`/`--deck-b`). Lets you measure whether MCTS can
    /// overcome a deck-quality gap — if it still wins above 50%, the
    /// AI advantage is independent of deck strength. Saved fitness is
    /// local-to-its-evolution-run so this is a rough proxy, not a
    /// strict ordering; run `make curate-baselines` for fresh ranks.
    #[arg(long = "handicap", default_value_t = false)]
    pub handicap: bool,
}

/// Run a single game with `ai_a` driving player A on `deck_a` and
/// `ai_b` driving player B on `deck_b`. Returns the winning PlayerId.
fn play_one(
    registry: &CardRegistry,
    deck_a: &[Card],
    deck_b: &[Card],
    game_seed: u64,
    ais: &[AiKind; 2],
) -> PlayerId {
    let mut state = GameState::new(deck_a.to_vec(), deck_b.to_vec());
    state.replay_journal = Some(tsot::game::Journal::new());
    let mut rng = StdRng::seed_from_u64(game_seed);
    let mut log: Vec<String> = Vec::new();
    let stats = run_game_continue(&mut state, &mut rng, &mut log, registry.lua(), ais);
    stats.winner
}

/// Load one EvolvedDeck JSON into a `(Vec<Card>, label, fitness)` triple.
/// `fitness` is the saved local-to-evolution-run fitness — useful as a
/// rough deck-strength proxy for the handicap flag, not as an absolute
/// ranking across runs.
fn load_deck(
    registry: &CardRegistry,
    path: &str,
) -> mlua::Result<(Vec<Card>, String, f64)> {
    let saved = EvolvedDeck::load(std::path::Path::new(path))
        .map_err(|e| mlua::Error::runtime(format!("load deck {path}: {e}")))?;
    let cards = saved
        .to_cards(registry)
        .map_err(|e| mlua::Error::runtime(format!("materialize deck {path}: {e}")))?;
    Ok((
        cards,
        format!("{path} (label={}, fitness={:.3})", saved.label, saved.fitness),
        saved.fitness,
    ))
}

pub fn run_matchup_mcts(
    registry: &CardRegistry,
    playable_pool: &[Card],
    args: &MatchupMctsArgs,
) -> mlua::Result<()> {
    let mut rng = StdRng::seed_from_u64(args.seed);
    let mcts_cfg = MctsConfig {
        rollouts_per_candidate: args.rollouts_per_candidate,
        max_candidates: args.max_candidates,
        base_seed: args.mcts_seed,
    };
    let ai_h = AiKind::Heuristic;
    let ai_m = AiKind::Mcts(mcts_cfg);

    println!();
    println!("=== matchup-mcts ===");
    println!(
        "  games (per side)={} total={} | rollouts={} max-candidates={} | seed={:#x} mcts-seed={:#x}",
        args.games,
        args.games * 2,
        args.rollouts_per_candidate,
        args.max_candidates,
        args.seed,
        args.mcts_seed,
    );
    println!();

    let mut mcts_wins: u32 = 0;
    let mut heuristic_wins: u32 = 0;
    mcts::reset_mcts_diagnostics();
    let t_start = std::time::Instant::now();

    // Deck loading. Three resolution paths in priority order:
    //   1. EXPLICIT ASYMMETRIC: both `--deck-a` and `--deck-b` set.
    //   2. EXPLICIT MIRROR: `--deck` set.
    //   3. DEFAULT: if `baselines/` has ≥2 deck JSONs, run asymmetric
    //      with the first two (sorted). If only 1, mirror that deck.
    //      If none, fall back to a random-genome mirror.
    //
    // Asymmetric is the default because it surfaces "AI × deck"
    // interactions instead of pure AI strength. Override with `--deck`
    // to force a mirror.
    let (mcts_deck, heuristic_deck, deck_label) = if args.deck_a.is_some()
        && args.deck_b.is_some()
    {
        let (a, la, _) = load_deck(registry, args.deck_a.as_ref().unwrap())?;
        let (b, lb, _) = load_deck(registry, args.deck_b.as_ref().unwrap())?;
        (a, b, format!("asymmetric — MCTS plays {la}; Heuristic plays {lb}"))
    } else if let Some(path) = &args.deck {
        let (deck, label, _) = load_deck(registry, path)?;
        (deck.clone(), deck, format!("mirror — {label}"))
    } else {
        // Auto-pick from baselines/.
        let mut baseline_paths: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir("baselines") {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("json") {
                    baseline_paths.push(p);
                }
            }
        }
        // Stable sort first so the random shuffle below is reproducible
        // from the master seed regardless of filesystem read order.
        baseline_paths.sort();
        match baseline_paths.len() {
            0 => {
                let genome = random_genome(playable_pool, 50, 3, &mut rng)
                    .map_err(|e| mlua::Error::runtime(format!("random_genome: {e}")))?;
                let cards = to_deck(registry, &genome)
                    .map_err(|e| mlua::Error::runtime(format!("to_deck: {e}")))?;
                (cards.clone(), cards, "mirror — random-genome (no baselines/)".to_string())
            }
            1 => {
                let (deck, label, _) = load_deck(registry, &baseline_paths[0].to_string_lossy())?;
                (deck.clone(), deck, format!("mirror (only 1 baseline) — {label}"))
            }
            _ => {
                // Pick two distinct random baselines using the master rng.
                // Seed-derived so the same `--seed` reproduces the same pairing.
                use rand::seq::SliceRandom;
                baseline_paths.shuffle(&mut rng);
                let (deck_x, lx, fx) = load_deck(registry, &baseline_paths[0].to_string_lossy())?;
                let (deck_y, ly, fy) = load_deck(registry, &baseline_paths[1].to_string_lossy())?;
                // Handicap: MCTS gets the lower-fitness deck.
                // Otherwise: random assignment (first shuffled → MCTS).
                let (mcts_d, mcts_l, heur_d, heur_l, mode_note) = if args.handicap {
                    if fx < fy {
                        (deck_x, lx, deck_y, ly, "HANDICAP — MCTS gets the lower-fitness deck")
                    } else {
                        (deck_y, ly, deck_x, lx, "HANDICAP — MCTS gets the lower-fitness deck")
                    }
                } else {
                    (deck_x, lx, deck_y, ly, "default — random pairing")
                };
                (
                    mcts_d,
                    heur_d,
                    format!("asymmetric ({mode_note}) — MCTS plays {mcts_l}; Heuristic plays {heur_l}"),
                )
            }
        }
    };
    println!(
        "  deck: {deck_label} (MCTS={} cards, Heuristic={} cards)",
        mcts_deck.len(),
        heuristic_deck.len()
    );

    // Round 1: MCTS on A, Heuristic on B.
    // Asymmetric mode: MCTS's deck on A, Heuristic's deck on B.
    // Mirror mode: same deck both sides (mcts_deck == heuristic_deck).
    println!();
    println!("--- MCTS on A, Heuristic on B ---");
    let ais_ma = [ai_m.clone(), ai_h.clone()];
    for g in 0..args.games {
        let game_seed = rng.gen();
        let winner = play_one(registry, &mcts_deck, &heuristic_deck, game_seed, &ais_ma);
        let label = if winner == PlayerId::A {
            mcts_wins += 1;
            "MCTS wins"
        } else {
            heuristic_wins += 1;
            "Heuristic wins"
        };
        println!(
            "  game {:>2}/{} (seed={:#x}) → {label}",
            g + 1,
            args.games,
            game_seed
        );
    }

    // Round 2: Heuristic on A, MCTS on B.
    // Asymmetric mode: Heuristic's deck on A, MCTS's deck on B
    // (so MCTS keeps its deck while sides swap to cancel first-mover
    // advantage).
    println!();
    println!("--- Heuristic on A, MCTS on B ---");
    let ais_mb = [ai_h, ai_m];
    for g in 0..args.games {
        let game_seed = rng.gen();
        let winner = play_one(registry, &heuristic_deck, &mcts_deck, game_seed, &ais_mb);
        let label = if winner == PlayerId::B {
            mcts_wins += 1;
            "MCTS wins"
        } else {
            heuristic_wins += 1;
            "Heuristic wins"
        };
        println!(
            "  game {:>2}/{} (seed={:#x}) → {label}",
            g + 1,
            args.games,
            game_seed
        );
    }

    let elapsed = t_start.elapsed();
    let total = mcts_wins + heuristic_wins;
    let mcts_rate = mcts_wins as f64 / total as f64;

    let pick_calls = MCTS_PICK_CALLS.load(Ordering::SeqCst);
    let searched = MCTS_SEARCHED_PICKS.load(Ordering::SeqCst);
    let total_cands = MCTS_TOTAL_CANDIDATES.load(Ordering::SeqCst);
    let avg_cands = if pick_calls == 0 {
        0.0
    } else {
        total_cands as f64 / pick_calls as f64
    };

    println!();
    println!("=== summary ===");
    println!(
        "  MCTS wins:      {mcts_wins}/{total}  ({:.1}%)",
        mcts_rate * 100.0
    );
    println!(
        "  Heuristic wins: {heuristic_wins}/{total}  ({:.1}%)",
        (1.0 - mcts_rate) * 100.0
    );
    println!("  wall: {:.1?}  ({:.1?}/game)", elapsed, elapsed / total);
    println!();
    println!("=== MCTS diagnostics ===");
    println!("  pick_play calls:     {pick_calls}");
    println!("  searched (>1 cand):  {searched} ({:.1}%)",
        if pick_calls > 0 { searched as f64 / pick_calls as f64 * 100.0 } else { 0.0 });
    println!("  avg candidates/call: {avg_cands:.2}");
    if searched == 0 && pick_calls > 0 {
        println!("  ⚠  MCTS short-circuited every pick (≤1 candidate per turn).");
        println!("     The 50/50 result is artificial — both AIs picked identically.");
        println!("     Try a richer deck via `--deck baselines/<file>.json`.");
    }

    Ok(())
}
