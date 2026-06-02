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
    /// if unset.
    #[arg(long = "deck", value_name = "PATH")]
    pub deck: Option<String>,
}

/// Run a single mirror-match game with `ai_a` driving player A and
/// `ai_b` driving player B. Returns the winning PlayerId.
fn play_one(
    registry: &CardRegistry,
    deck: &[Card],
    game_seed: u64,
    ais: &[AiKind; 2],
) -> PlayerId {
    let mut state = GameState::new(deck.to_vec(), deck.to_vec());
    state.replay_journal = Some(tsot::game::Journal::new());
    let mut rng = StdRng::seed_from_u64(game_seed);
    let mut log: Vec<String> = Vec::new();
    let stats = run_game_continue(&mut state, &mut rng, &mut log, registry.lua(), ais);
    stats.winner
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

    // Load deck. Prefer `--deck PATH`; fall back to first baseline;
    // fall back to random-genome (noted as a poor mirror-match shape).
    let (deck, deck_label, deck_size_unique) = if let Some(path) = &args.deck {
        let saved = EvolvedDeck::load(std::path::Path::new(path))
            .map_err(|e| mlua::Error::runtime(format!("load deck {path}: {e}")))?;
        let cards = saved
            .to_cards(registry)
            .map_err(|e| mlua::Error::runtime(format!("materialize deck {path}: {e}")))?;
        let unique = saved
            .card_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        (cards, format!("{path} (label={})", saved.label), unique)
    } else if let Ok(rd) = std::fs::read_dir("baselines") {
        let mut first: Option<std::path::PathBuf> = None;
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                if first.is_none() || p < *first.as_ref().unwrap() {
                    first = Some(p);
                }
            }
        }
        if let Some(p) = first {
            let saved = EvolvedDeck::load(&p)
                .map_err(|e| mlua::Error::runtime(format!("load baseline {}: {e}", p.display())))?;
            let cards = saved
                .to_cards(registry)
                .map_err(|e| mlua::Error::runtime(format!("materialize baseline {}: {e}", p.display())))?;
            let unique = saved
                .card_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            (cards, format!("{} (label={})", p.display(), saved.label), unique)
        } else {
            // No baselines available — fall back to random-genome.
            let genome = random_genome(playable_pool, 50, 3, &mut rng)
                .map_err(|e| mlua::Error::runtime(format!("random_genome: {e}")))?;
            let cards = to_deck(registry, &genome)
                .map_err(|e| mlua::Error::runtime(format!("to_deck: {e}")))?;
            let unique = genome
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            (cards, "random-genome (no baselines/ found)".to_string(), unique)
        }
    } else {
        let genome = random_genome(playable_pool, 50, 3, &mut rng)
            .map_err(|e| mlua::Error::runtime(format!("random_genome: {e}")))?;
        let cards = to_deck(registry, &genome)
            .map_err(|e| mlua::Error::runtime(format!("to_deck: {e}")))?;
        let unique = genome
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        (cards, "random-genome (no baselines/ found)".to_string(), unique)
    };
    println!("  deck: {deck_label} ({} cards, {deck_size_unique} unique)", deck.len());

    // Round 1: MCTS on A, Heuristic on B.
    println!();
    println!("--- MCTS on A, Heuristic on B ---");
    let ais_ma = [ai_m.clone(), ai_h.clone()];
    for g in 0..args.games {
        let game_seed = rng.gen();
        let winner = play_one(registry, &deck, game_seed, &ais_ma);
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
    println!();
    println!("--- Heuristic on A, MCTS on B ---");
    let ais_mb = [ai_h, ai_m];
    for g in 0..args.games {
        let game_seed = rng.gen();
        let winner = play_one(registry, &deck, game_seed, &ais_mb);
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
