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

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use clap::Parser;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

use tsot::card::{Card, CardRegistry};
use tsot::game::{GameState, PlayerId};
use tsot::sim::stats::GameStats;

use crate::parse_u64_hex_or_dec;
use tsot::sim;
use tsot::sim::genome::{random_genome, to_deck};

thread_local! {
    /// Per-thread CardRegistry. `mlua::Lua` is `!Send`, so each rayon
    /// worker thread loads its own copy of the cards (~100ms first
    /// touch, then reused). Determinism: each game's RNG is seeded
    /// from a per-game seed derived serially from the master rng, so
    /// outcomes are independent of which worker ran them.
    static WORKER_REGISTRY: RefCell<Option<Arc<CardRegistry>>> = const { RefCell::new(None) };
}

fn worker_registry() -> Arc<CardRegistry> {
    WORKER_REGISTRY.with(|cell| {
        let mut r = cell.borrow_mut();
        if r.is_none() {
            *r = Some(Arc::new(
                CardRegistry::load_embedded().expect("worker: load_embedded failed"),
            ));
        }
        r.as_ref().unwrap().clone()
    })
}

/// Per-game work item — the slice each worker processes.
struct GameSpec {
    g: u32,
    genome_a: Vec<String>,
    genome_b: Vec<String>,
    seed: u64,
}

/// Per-game result aggregated after the parallel section.
struct GameOutcome {
    g: u32,
    elapsed: std::time::Duration,
    play_events: Vec<(String, u32, PlayerId)>,
}

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

    use tsot::sim::instrument::{
        paint_bold_green, paint_cyan, paint_dim, paint_green, paint_yellow,
    };
    println!();
    println!("{}", paint_cyan("=== curve-sample ==="));
    println!(
        "  {} games={} seed={:#x}",
        paint_dim("·"),
        args.games,
        args.seed
    );
    println!(
        "  {} pool size: {} cards",
        paint_dim("·"),
        playable_pool.len()
    );
    println!("  {} ai: {:?} (both seats)", paint_dim("·"), ai_kind);
    println!();

    // Phase 1: serial job generation. Pre-roll every game's
    // (seed, genome_a, genome_b) from the master rng so the input
    // sequence is deterministic regardless of how many worker threads
    // rayon spins up. Each worker then runs purely from the spec.
    let mut specs: Vec<GameSpec> = Vec::with_capacity(args.games as usize);
    for g in 0..args.games {
        let seed: u64 = rng.gen();
        let genome_a = random_genome(playable_pool, 50, 3, &mut rng)
            .map_err(|e| mlua::Error::runtime(format!("random_genome A: {e}")))?;
        let genome_b = random_genome(playable_pool, 50, 3, &mut rng)
            .map_err(|e| mlua::Error::runtime(format!("random_genome B: {e}")))?;
        specs.push(GameSpec {
            g: g + 1,
            genome_a,
            genome_b,
            seed,
        });
    }

    let _ = registry;  // unused in parallel path — each worker uses its thread-local

    let t_start = std::time::Instant::now();

    // Phase 2: parallel game execution. Each worker uses a thread-
    // local CardRegistry (mlua::Lua is !Send), materializes its decks
    // from the genome string ids, runs the game, and emits the
    // formatted output as a single `eprint!` call so the START/END
    // chunk stays contiguous on stderr. Output order is by completion
    // time, not game number — that's the rayon trade-off.
    let outcomes: Vec<GameOutcome> = specs
        .into_par_iter()
        .map(|spec| {
            let reg = worker_registry();
            let deck_a = to_deck(&reg, &spec.genome_a).expect("to_deck A");
            let deck_b = to_deck(&reg, &spec.genome_b).expect("to_deck B");
            let state = GameState::new(deck_a, deck_b);
            let mut game_rng = StdRng::seed_from_u64(spec.seed);
            let mut log: Vec<String> = Vec::new();
            // Clear any failure entries lingering from a prior game
            // this thread happened to run. Rayon reuses worker
            // threads, so the thread-local sink could have residuals.
            let _ = tsot::sim::instrument::drain_failures();
            let game_t0 = std::time::Instant::now();
            let (stats, _) =
                sim::run_game_with_ai(state, &mut game_rng, &mut log, &reg, &ais, spec.seed);
            let game_elapsed = game_t0.elapsed();
            // Fold every failure message this game produced into the
            // game's engine log. The failed-game classifier picks
            // them up (they contain "failed:" / "rollout-stall" /
            // "[play_card-ERR]" / "[NoHandPaymentForIdentity]") and
            // the failed-game dump surfaces them at game-end as one
            // batched write.
            log.extend(tsot::sim::instrument::drain_failures());

            // Format every line into one String so the whole game's
            // output is one atomic write (println-lock guarantees no
            // interleaving with other threads).
            let buf = format_game_output(spec.g, args.games, spec.seed, &stats, &log, game_elapsed);
            eprint!("{buf}");

            GameOutcome {
                g: spec.g,
                elapsed: game_elapsed,
                play_events: stats.card_play_turn_events,
            }
        })
        .collect();

    // Phase 3: serial aggregation.
    let mut slowest: (u32, std::time::Duration) = (0, std::time::Duration::ZERO);
    for o in &outcomes {
        if o.elapsed > slowest.1 {
            slowest = (o.g, o.elapsed);
        }
        for (card_id, turn, _player) in &o.play_events {
            *acc.entry(card_id.clone()).or_default().entry(*turn).or_insert(0) += 1;
            total_plays += 1;
        }
    }
    let total_elapsed = t_start.elapsed();
    eprintln!(
        "  {}  {} games  total {}  avg {}/game  slowest {} at {}  {} plays",
        paint_bold_green("∎ done"),
        args.games,
        paint_yellow(format!("{total_elapsed:>5.1?}")),
        paint_dim(format!(
            "{:>4.1}s",
            total_elapsed.as_secs_f64() / args.games as f64
        )),
        paint_dim(format!("#{}", slowest.0)),
        paint_yellow(format!("{:.2?}", slowest.1)),
        total_plays,
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

    // Stdout summary: top 15 cards played on turn 1, turn 2, and
    // turn 3 each. Reads the same `acc` map (card_id → turn →
    // count) — for each target turn N, rank cards by their count at
    // turn N descending. Shows the early-game cast distribution
    // directly instead of mean/median which can mask early-vs-late
    // bimodality.
    for target_turn in [1u32, 2, 3] {
        println!();
        println!(
            "{}",
            paint_cyan(format!("=== top 15 cards played on turn {target_turn} ==="))
        );
        let mut ranked: Vec<(&String, u32)> = acc
            .iter()
            .filter_map(|(id, by_turn)| {
                let n = *by_turn.get(&target_turn).unwrap_or(&0);
                if n > 0 { Some((id, n)) } else { None }
            })
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        if ranked.is_empty() {
            println!("  {}", paint_dim("(no plays recorded on this turn)"));
            continue;
        }
        for (id, plays_on_turn) in ranked.iter().take(15) {
            let total: u32 = acc.get(*id).map(|m| m.values().sum()).unwrap_or(0);
            let share_pct = if total > 0 {
                (*plays_on_turn as f64) * 100.0 / (total as f64)
            } else {
                0.0
            };
            // Highlight cards with a high share (this turn is their
            // primary cast turn) in green; the rest stay default.
            let id_painted = if share_pct >= 25.0 {
                paint_green(format!("{id:<32}"))
            } else {
                format!("{id:<32}")
            };
            println!(
                "  {}  {} {}  {}  {}",
                id_painted,
                paint_yellow(format!("plays_t{target_turn}=")),
                paint_yellow(format!("{plays_on_turn:>3}")),
                paint_dim(format!("share={share_pct:>5.1}%")),
                paint_dim(format!("total={total:>4}")),
            );
        }
    }

    Ok(())
}

/// Format one game's full output as a single contiguous String. The
/// caller (the rayon worker) emits this via one `eprint!` so the
/// chunk doesn't interleave with other workers' output.
fn format_game_output(
    g: u32,
    total_games: u32,
    seed: u64,
    stats: &GameStats,
    log: &[String],
    elapsed: std::time::Duration,
) -> String {
    use std::fmt::Write;
    use tsot::sim::instrument::{
        paint_blue, paint_bold_green, paint_bold_red, paint_dim, paint_green, paint_red,
        paint_yellow,
    };
    // A game is FAILED if any play_card returned Err, or if any
    // rollout stalled. Failures are signal — they get the full
    // engine log dumped below, always.
    let failed = log.iter().any(|line| {
        line.contains("failed:")
            || line.contains("rollout-stall")
            || line.contains("[play_card-ERR]")
            || line.contains("[NoHandPaymentForIdentity]")
    });
    let end_tag = if failed {
        paint_bold_red("✗ END  ")
    } else {
        paint_bold_green("● END  ")
    };
    let took_painted = if elapsed.as_secs_f64() > 2.0 {
        paint_yellow(format!("took {elapsed:>7.2?}"))
    } else {
        paint_dim(format!("took {elapsed:>7.2?}"))
    };
    let winner_painted = match stats.winner {
        PlayerId::A => paint_green("winner=A"),
        PlayerId::B => paint_blue("winner=B"),
    };
    let seed_painted = paint_dim(format!("seed={seed:#x}"));
    let mut buf = String::new();
    let _ = writeln!(
        buf,
        "  {} game {g:>4}/{total_games}  {}  turns={}  {}  {}  {}",
        end_tag,
        took_painted,
        stats.turns,
        winner_painted,
        seed_painted,
        paint_dim(format!(
            "plays(A/B)={}/{}  atks(A/B)={}/{}  deaths(A/B)={}/{}  mill(A/B)={}/{}  board(A/B)={}/{}  gy(A/B)={}/{}  rj={}",
            stats.a_played, stats.b_played,
            stats.a_attacks, stats.b_attacks,
            stats.a_deaths, stats.b_deaths,
            stats.a_milled_to_exile, stats.b_milled_to_exile,
            stats.a_final_board, stats.b_final_board,
            stats.a_final_gy, stats.b_final_gy,
            stats.replay_journal_entries,
        )),
    );
    if !failed {
        return buf;
    }
    let fires: Vec<String> = stats
        .event_fires
        .iter()
        .filter(|(_, [a, b])| a + b > 0)
        .map(|(k, [a, b])| format!("{k:?}={a}/{b}"))
        .collect();
    let actions: Vec<String> = stats
        .action_counts
        .iter()
        .filter(|(_, [a, b])| a + b > 0)
        .map(|(k, [a, b])| format!("{k}={a}/{b}"))
        .collect();
    if !fires.is_empty() {
        let _ = writeln!(buf, "            event_fires(A/B): {}", fires.join("  "));
    }
    if !actions.is_empty() {
        let _ = writeln!(buf, "            actions(A/B):     {}", actions.join("  "));
    }
    if !stats.card_sacrificed_count.is_empty() {
        let s: Vec<String> = stats
            .card_sacrificed_count
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let _ = writeln!(buf, "            sacrificed:       {}", s.join("  "));
    }
    if !stats.card_discarded_count.is_empty() {
        let s: Vec<String> = stats
            .card_discarded_count
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let _ = writeln!(buf, "            discarded:        {}", s.join("  "));
    }
    let _ = writeln!(buf, "            | full engine log ({} lines):", log.len());
    for line in log {
        let painted = if line.contains("failed:") {
            paint_red(line)
        } else if line.contains("rollout-stall") {
            paint_yellow(line)
        } else {
            paint_dim(line)
        };
        let _ = writeln!(buf, "            | {painted}");
    }
    buf
}
