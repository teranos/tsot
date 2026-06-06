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
use tsot::sim;
use tsot::sim::genome::{random_genome, to_deck};

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
    /// AI for both seats. `uct` (default) gives high-signal play so
    /// the turn-played histograms reflect real card-driven timing.
    /// `heuristic` is the legacy fast option.
    #[arg(long = "opponent-ai", default_value = "uct")]
    pub opponent_ai: String,
    /// UCT iterations per pick when `--opponent-ai uct`.
    #[arg(long = "opponent-uct-iterations", default_value_t = 10)]
    pub opponent_uct_iterations: u32,
    /// UCT exploration constant when `--opponent-ai uct`. `sqrt(2)`
    /// is classical.
    #[arg(long = "opponent-uct-c", default_value_t = std::f64::consts::SQRT_2)]
    pub opponent_uct_c: f64,
}

pub fn run_curve_sample(
    registry: &std::sync::Arc<CardRegistry>,
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

    let ai_kind = match args.opponent_ai.to_ascii_lowercase().as_str() {
        "heuristic" => tsot::sim::AiKind::Heuristic,
        "uct" => tsot::sim::AiKind::Uct(tsot::sim::uct::UctConfig {
            iterations: args.opponent_uct_iterations,
            exploration_c: args.opponent_uct_c,
            ..Default::default()
        }),
        other => {
            eprintln!("error: --opponent-ai must be 'heuristic' | 'uct', got {other:?}");
            std::process::exit(2);
        }
    };
    let ais = [ai_kind.clone(), ai_kind.clone()];

    println!();
    println!("=== curve-sample ===");
    println!("  games={} seed={:#x}", args.games, args.seed);
    println!("  pool size: {} cards", playable_pool.len());
    println!("  ai: {:?} (both seats)", ai_kind);
    println!();

    // Observability principle: print everything that exists. No
    // thresholds, no `\r` overwrites, no truncation. Each game emits
    // permanent START + END lines. A watchdog heartbeats every second
    // for whatever game is currently running so the operator sees
    // motion even within a single slow game. A side file
    // `target/curve-sample-current.json` is written on every game's
    // START with the seed and both full genomes, so a kill-and-read
    // workflow always shows which decks the runner is on.
    type CurrentGame = (u32, std::time::Instant, Vec<String>, Vec<String>, u64);
    let current_game: std::sync::Arc<std::sync::Mutex<Option<CurrentGame>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let watch = current_game.clone();
    let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watch_shutdown = shutdown.clone();
    let watchdog = std::thread::spawn(move || {
        while !watch_shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let snap = watch.lock().ok().and_then(|g| g.clone());
            if let Some((gnum, started, _, _, _)) = snap {
                let elapsed = started.elapsed();
                let op = tsot::sim::instrument::current_op();
                eprintln!(
                    "  [heartbeat] game {gnum} elapsed {:.1?}  current_op={op}",
                    elapsed,
                );
            }
        }
    });

    let t_start = std::time::Instant::now();
    let mut slowest: (u32, std::time::Duration) = (0, std::time::Duration::ZERO);
    let _ = std::fs::create_dir_all("target");
    for g in 0..args.games {
        let running = g + 1;
        let game_seed: u64 = rng.gen();
        let genome_a = random_genome(playable_pool, 50, 3, &mut rng)
            .map_err(|e| mlua::Error::runtime(format!("random_genome A: {e}")))?;
        let genome_b = random_genome(playable_pool, 50, 3, &mut rng)
            .map_err(|e| mlua::Error::runtime(format!("random_genome B: {e}")))?;

        // Side file with the FULL genomes and seed — written before
        // the game runs so a kill at any moment leaves the operator
        // pointing at the exact pair that reproduces the slowness.
        let dump_path = "target/curve-sample-current.json";
        let dump = format!(
            "{{\n  \"game\": {running},\n  \"seed\": \"{game_seed:#x}\",\n  \"genome_a\": {},\n  \"genome_b\": {}\n}}\n",
            serde_json::to_string(&genome_a).unwrap_or_else(|_| "[]".to_string()),
            serde_json::to_string(&genome_b).unwrap_or_else(|_| "[]".to_string()),
        );
        let _ = std::fs::write(dump_path, dump);

        // Hand the watchdog the live state.
        {
            let mut slot = current_game.lock().unwrap();
            *slot = Some((running, std::time::Instant::now(), genome_a.clone(), genome_b.clone(), game_seed));
        }

        let elapsed_so_far = t_start.elapsed();
        let eta_so_far = if g == 0 {
            "?".to_string()
        } else {
            let per_game = elapsed_so_far.as_secs_f64() / g as f64;
            format!("{:.0}s", per_game * (args.games - g) as f64)
        };
        eprintln!(
            "  game {running:>4}/{} START  seed={game_seed:#x}  elapsed_total {:>5.1?}  eta {eta_so_far:>5}  decks_dumped_to={dump_path}",
            args.games, elapsed_so_far,
        );

        let deck_a = to_deck(registry, &genome_a)
            .map_err(|e| mlua::Error::runtime(format!("to_deck A: {e}")))?;
        let deck_b = to_deck(registry, &genome_b)
            .map_err(|e| mlua::Error::runtime(format!("to_deck B: {e}")))?;
        let state = GameState::new(deck_a, deck_b);
        let mut game_rng = StdRng::seed_from_u64(game_seed);
        let mut log: Vec<String> = Vec::new();
        let game_t0 = std::time::Instant::now();
        let (stats, _) = sim::run_game_with_ai(state, &mut game_rng, &mut log, registry, &ais);
        let game_elapsed = game_t0.elapsed();

        *current_game.lock().unwrap() = None;

        if game_elapsed > slowest.1 {
            slowest = (running, game_elapsed);
        }

        // Full per-game readout. Every field GameStats exposes about
        // the finished game shows up here — winner, turns, per-side
        // play/attack/death/mill totals, final board+GY sizes, the
        // event-fire and action-count maps. Hiding any of this would
        // mean making the operator dig for it later when something
        // looks weird.
        let winner_str = match stats.winner {
            tsot::game::PlayerId::A => "A",
            tsot::game::PlayerId::B => "B",
        };
        eprintln!(
            "  game {running:>4}/{} END    took {:>7.2?}  turns={}  winner={winner_str}  plays(A/B)={}/{}  atks(A/B)={}/{}  deaths(A/B)={}/{}  mill_to_exile(A/B)={}/{}  final_board(A/B)={}/{}  final_gy(A/B)={}/{}  replay_journal={}",
            args.games, game_elapsed, stats.turns,
            stats.a_played, stats.b_played,
            stats.a_attacks, stats.b_attacks,
            stats.a_deaths, stats.b_deaths,
            stats.a_milled_to_exile, stats.b_milled_to_exile,
            stats.a_final_board, stats.b_final_board,
            stats.a_final_gy, stats.b_final_gy,
            stats.replay_journal_entries,
        );
        // Lua-handler events and engine action counts: print only the
        // entries that actually fired (zeros are noise). Nothing
        // truncated within the non-zero set.
        let fires: Vec<String> = stats.event_fires.iter()
            .filter(|(_, [a, b])| a + b > 0)
            .map(|(k, [a, b])| format!("{k:?}={a}/{b}"))
            .collect();
        let actions: Vec<String> = stats.action_counts.iter()
            .filter(|(_, [a, b])| a + b > 0)
            .map(|(k, [a, b])| format!("{k}={a}/{b}"))
            .collect();
        if !fires.is_empty() {
            eprintln!("            event_fires(A/B): {}", fires.join("  "));
        }
        if !actions.is_empty() {
            eprintln!("            actions(A/B):     {}", actions.join("  "));
        }
        // Sacrifices and discards, per-card. Show every entry.
        if !stats.card_sacrificed_count.is_empty() {
            let s: Vec<String> = stats.card_sacrificed_count.iter()
                .map(|(k, v)| format!("{k}={v}")).collect();
            eprintln!("            sacrificed:       {}", s.join("  "));
        }
        if !stats.card_discarded_count.is_empty() {
            let s: Vec<String> = stats.card_discarded_count.iter()
                .map(|(k, v)| format!("{k}={v}")).collect();
            eprintln!("            discarded:        {}", s.join("  "));
        }
        // Full action log for this game — written by the engine
        // during run_game_with_ai. We were discarding it silently;
        // that was hiding data. Print it under the END line so the
        // operator can read exactly what happened action-by-action.
        // The lines themselves come from the engine and are not
        // summarized or filtered here.
        for line in &log {
            eprintln!("            | {line}");
        }

        for (card_id, turn, _player) in &stats.card_play_turn_events {
            let by_turn = acc.entry(card_id.clone()).or_default();
            *by_turn.entry(*turn).or_insert(0) += 1;
            total_plays += 1;
        }
    }
    shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = watchdog.join();
    let total_elapsed = t_start.elapsed();
    eprintln!(
        "  {} games  | total {:>5.1?}  | avg {:>4.1}s/game  | slowest #{} at {:.2?}  | {total_plays} plays",
        args.games, total_elapsed,
        total_elapsed.as_secs_f64() / args.games as f64,
        slowest.0, slowest.1,
    );

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
