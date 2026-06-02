//! `tsot curve-sample` subcommand: plays N games of random-deck vs
//! random-deck, aggregates per-card play-turn distributions, dumps
//! `card-curve.json` for the `cards-report.py` dashboard. Both
//! players' plays are counted (the scope-2 choice — see the design
//! notes in cli_balance_probe).
//!
//! Random decks are the right sampling shape here: every card in the
//! pool has a roughly-uniform chance of appearing, so the resulting
//! per-card play-turn distribution isn't biased toward any one
//! archetype. ~200 games × 2 decks gives ~80-100 play events per
//! typical card — enough for a stable median.
//!
//! Output: `card-curve.json` — JSON to match the rest of the project's
//! on-disk data convention (champions/baselines are JSON too). Shape:
//!
//! ```text
//! {
//!   "n_games": 200,
//!   "seed": "0xc07e",
//!   "card_curves": {
//!     "hydra": {"plays": 245, "turns": {"3": 12, "4": 18}},
//!     ...
//!   }
//! }
//! ```
//!
//! Consumed by `tools/cards-report.py` via the stdlib `json` module.
//! The file is formatted with one card per line for human diffability.

use std::collections::BTreeMap;

use clap::Parser;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::{Card, CardRegistry};
use tsot::game::GameState;

use crate::parse_u64_hex_or_dec;
use crate::sim;
use crate::sim::genome::{random_genome, to_deck};

#[derive(Parser)]
pub struct CurveSampleArgs {
    /// Number of games to play. Each game pairs two random-genome
    /// decks. Both decks' plays count toward the aggregate.
    #[arg(long, default_value_t = 200)]
    pub games: u32,
    /// Master seed.
    #[arg(long, default_value_t = 0xC0_7E, value_parser = parse_u64_hex_or_dec)]
    pub seed: u64,
    /// Output path. Default `card-curve.json` — consumed by
    /// `tools/cards-report.py` via the stdlib `json` module.
    #[arg(long = "out", default_value = "card-curve.json")]
    pub out: String,
}

pub fn run_curve_sample(
    registry: &CardRegistry,
    playable_pool: &[Card],
    args: &CurveSampleArgs,
) -> mlua::Result<()> {
    if playable_pool.is_empty() {
        eprintln!("error: playable_pool is empty");
        std::process::exit(2);
    }

    let mut rng = StdRng::seed_from_u64(args.seed);
    // Accumulate `card_id → turn → count` across every game.
    let mut acc: BTreeMap<String, BTreeMap<u32, u32>> = BTreeMap::new();
    let mut total_plays: u64 = 0;

    println!();
    println!("=== curve-sample ===");
    println!("  games={} seed={:#x}", args.games, args.seed);
    println!("  pool size: {} cards", playable_pool.len());
    println!();

    let t_start = std::time::Instant::now();
    for g in 0..args.games {
        let genome_a = random_genome(playable_pool, 50, 3, &mut rng)
            .map_err(|e| mlua::Error::runtime(format!("random_genome A: {e}")))?;
        let genome_b = random_genome(playable_pool, 50, 3, &mut rng)
            .map_err(|e| mlua::Error::runtime(format!("random_genome B: {e}")))?;
        let deck_a = to_deck(registry, &genome_a)
            .map_err(|e| mlua::Error::runtime(format!("to_deck A: {e}")))?;
        let deck_b = to_deck(registry, &genome_b)
            .map_err(|e| mlua::Error::runtime(format!("to_deck B: {e}")))?;
        let state = GameState::new(deck_a, deck_b);
        let mut game_rng = StdRng::seed_from_u64(rng.gen());
        let mut log: Vec<String> = Vec::new();
        let (stats, _) = sim::run_game(state, &mut game_rng, &mut log, registry.lua());
        // Scope-2: sum across both players. The player field on each
        // event is preserved upstream for a future scope-3 consumer.
        for (card_id, turn, _player) in &stats.card_play_turn_events {
            let by_turn = acc.entry(card_id.clone()).or_default();
            *by_turn.entry(*turn).or_insert(0) += 1;
            total_plays += 1;
        }
        if (g + 1) % 50 == 0 || g + 1 == args.games {
            let elapsed = t_start.elapsed();
            println!(
                "  game {:>4}/{}  | elapsed {:>5.1?} | {total_plays} play events recorded",
                g + 1,
                args.games,
                elapsed,
            );
        }
    }
    println!();

    // JSON output, hand-formatted so each card lives on its own line.
    // Line-per-card shape kept for human diffability.
    let mut buf = String::new();
    buf.push_str("{\n");
    buf.push_str(&format!("  \"n_games\": {},\n", args.games));
    buf.push_str(&format!("  \"seed\": \"{:#x}\",\n", args.seed));
    buf.push_str("  \"card_curves\": {\n");
    let n_cards = acc.len();
    for (i, (card_id, by_turn)) in acc.iter().enumerate() {
        let plays: u32 = by_turn.values().sum();
        let mut turns_str = String::new();
        let mut first = true;
        for (t, c) in by_turn {
            if !first {
                turns_str.push_str(", ");
            }
            first = false;
            turns_str.push_str(&format!("\"{t}\": {c}"));
        }
        let comma = if i + 1 < n_cards { "," } else { "" };
        buf.push_str(&format!(
            "    \"{card_id}\": {{\"plays\": {plays}, \"turns\": {{{turns_str}}}}}{comma}\n",
        ));
    }
    buf.push_str("  }\n}\n");
    std::fs::write(&args.out, buf)
        .map_err(|e| mlua::Error::runtime(format!("write {}: {e}", args.out)))?;
    println!("→ wrote {}", args.out);

    // Quick stdout summary: top 15 most-played cards with median turn.
    println!();
    println!("=== top 15 most-played cards (median turn) ===");
    let mut summary: Vec<(&String, &BTreeMap<u32, u32>)> = acc.iter().collect();
    summary.sort_by(|a, b| {
        let a_total: u32 = a.1.values().sum();
        let b_total: u32 = b.1.values().sum();
        b_total.cmp(&a_total)
    });
    for (id, by_turn) in summary.iter().take(15) {
        let plays: u32 = by_turn.values().sum();
        let mut all_turns: Vec<u32> = Vec::new();
        for (t, c) in *by_turn {
            for _ in 0..*c {
                all_turns.push(*t);
            }
        }
        all_turns.sort();
        let median = if all_turns.is_empty() {
            0.0
        } else {
            let m = all_turns.len() / 2;
            if all_turns.len().is_multiple_of(2) {
                (all_turns[m - 1] + all_turns[m]) as f64 / 2.0
            } else {
                all_turns[m] as f64
            }
        };
        let mean: f64 = if plays == 0 {
            0.0
        } else {
            all_turns.iter().map(|t| *t as f64).sum::<f64>() / plays as f64
        };
        println!(
            "  {:<32}  plays={:>4}  median_turn={:.1}  mean_turn={:.2}",
            id, plays, median, mean,
        );
    }

    Ok(())
}
