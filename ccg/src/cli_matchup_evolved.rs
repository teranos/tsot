//! `tsot matchup-evolved` subcommand: round-robin grid over a deck
//! directory. Plays each ordered (A, B) pair for N games, writes a
//! heat-colored HTML report with the same depth as the old variant
//! report (turn count, per-deck turn count, per-game averages, event
//! firings, action totals, future-sim telemetry, replay journal,
//! pending mechanics, top cards, interesting games, matrix, per-deck).

use clap::Parser;
use maud::{html, PreEscaped, DOCTYPE};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::CardRegistry;
use tsot::game::GameState;

use crate::parse_u64_hex_or_dec;
use crate::report_style;
use tsot::sim;
use tsot::sim::evolved_deck::EvolvedDeck;

#[derive(Parser)]
pub struct MatchupEvolvedArgs {
    /// Directory containing EvolvedDeck JSON files to use as the
    /// players in the round-robin grid.
    #[arg(long, default_value = "baselines")]
    pub dir: String,
    /// Games per ordered (A, B) cell. With N decks, total games =
    /// N × N × this. Default 50 matches the variant matchup grid.
    #[arg(long, default_value_t = 50)]
    pub games: u32,
    /// Master seed for per-game RNG seeding. Same seed → byte-
    /// identical grid.
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
    pub seed: u64,
    /// Write an HTML grid report to this path.
    #[arg(long, value_name = "PATH", default_value = "matchup-evolved.html")]
    pub html: String,
}

pub fn run_matchup_evolved(
    registry: &std::sync::Arc<CardRegistry>,
    args: &MatchupEvolvedArgs,
) -> mlua::Result<()> {
    let dir = std::path::Path::new(&args.dir);
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                paths.push(p);
            }
        }
    }
    paths.sort();
    if paths.is_empty() {
        eprintln!("error: no *.json files in {}", dir.display());
        std::process::exit(2);
    }

    let mut labels: Vec<String> = Vec::new();
    let mut decks: Vec<Vec<tsot::game::DeckUnit>> = Vec::new();
    for path in &paths {
        match EvolvedDeck::load(path) {
            Ok(saved) => match saved.to_units(registry) {
                Ok(units) => {
                    labels.push(saved.label.clone());
                    decks.push(units);
                }
                Err(e) => eprintln!("  ! {} unloadable: {e}", path.display()),
            },
            Err(e) => eprintln!("  ! {} unparseable: {e}", path.display()),
        }
    }
    let n = decks.len();
    println!();
    println!(
        "=== Matchup-evolved grid: {n} decks × {n} × {} games = {} total ===",
        args.games,
        n * n * args.games as usize
    );
    for (i, (label, path)) in labels.iter().zip(&paths).enumerate() {
        println!("  [{i}] {label:<20} ({})", path.display());
    }
    println!();

    let mut wins: Vec<Vec<u32>> = vec![vec![0; n]; n];
    let mut all_stats: Vec<sim::GameStats> = Vec::with_capacity(n * n * args.games as usize);
    let mut game_keys: Vec<(usize, usize)> = Vec::with_capacity(n * n * args.games as usize);
    let t0 = std::time::Instant::now();
    let mut rng = StdRng::seed_from_u64(args.seed);
    for i in 0..n {
        for j in 0..n {
            for _ in 0..args.games {
                let state = GameState::from_units(decks[i].clone(), decks[j].clone());
                let game_seed: u64 = rng.gen();
                let mut game_rng = StdRng::seed_from_u64(game_seed);
                let mut log: Vec<String> = Vec::new();
                let (stats, _) = sim::run_game(state, &mut game_rng, &mut log, registry, game_seed);
                if stats.winner == tsot::game::PlayerId::A {
                    wins[i][j] += 1;
                }
                all_stats.push(stats);
                game_keys.push((i, j));
            }
        }
    }
    let elapsed = t0.elapsed();

    let label_w = labels.iter().map(|s| s.len()).max().unwrap_or(8).max(8);
    let total_games = all_stats.len() as f64;
    let mut turn_values: Vec<u32> = all_stats.iter().map(|s| s.turns).collect();
    turn_values.sort_unstable();
    let turn_min = turn_values.first().copied().unwrap_or(0);
    let turn_max = turn_values.last().copied().unwrap_or(0);
    let turn_mean: f64 = turn_values.iter().sum::<u32>() as f64 / total_games;
    let turn_median = if turn_values.is_empty() {
        0
    } else {
        turn_values[turn_values.len() / 2]
    };
    fn avg(stats: &[sim::GameStats], f: impl Fn(&sim::GameStats) -> f64) -> f64 {
        stats.iter().map(f).sum::<f64>() / stats.len() as f64
    }

    println!();
    println!(
        "Turn count:  min {turn_min}   median {turn_median}   mean {turn_mean:.1}   max {turn_max}"
    );
    let mut per_deck_turns: Vec<(u32, u32, u32, u32)> = vec![(0, 0, u32::MAX, 0); n];
    for (idx, (i, j)) in game_keys.iter().enumerate() {
        let t = all_stats[idx].turns;
        for &k in &[*i, *j] {
            let entry = &mut per_deck_turns[k];
            entry.0 += t;
            entry.1 += 1;
            entry.2 = entry.2.min(t);
            entry.3 = entry.3.max(t);
        }
    }
    println!();
    println!("Per-deck turn count (this deck plays either seat):");
    println!(
        "  {:<w$}  {:>8}  {:>8}  {:>8}  {:>8}",
        "deck", "min", "mean", "median", "max",
        w = label_w
    );
    for (k, label) in labels.iter().enumerate().take(n) {
        let (sum, count, mn, mx) = per_deck_turns[k];
        let mean = if count > 0 { sum as f64 / count as f64 } else { 0.0 };
        let mut ts: Vec<u32> = game_keys
            .iter()
            .enumerate()
            .filter(|(_, (i, j))| *i == k || *j == k)
            .map(|(idx, _)| all_stats[idx].turns)
            .collect();
        ts.sort_unstable();
        let median = if ts.is_empty() { 0 } else { ts[ts.len() / 2] };
        println!(
            "  {:<w$}  {:>8}  {:>8.1}  {:>8}  {:>8}",
            label, mn, mean, median, mx,
            w = label_w
        );
    }

    println!();
    println!("Per-game averages (across {} games):", all_stats.len());
    println!("                       A           B");
    println!(
        "  cards played        {:>6.1}      {:>6.1}",
        avg(&all_stats, |s| s.a_played as f64),
        avg(&all_stats, |s| s.b_played as f64)
    );
    println!(
        "  attacks declared    {:>6.1}      {:>6.1}",
        avg(&all_stats, |s| s.a_attacks as f64),
        avg(&all_stats, |s| s.b_attacks as f64)
    );
    println!(
        "  deaths (own creat.) {:>6.1}      {:>6.1}",
        avg(&all_stats, |s| s.a_deaths as f64),
        avg(&all_stats, |s| s.b_deaths as f64)
    );
    println!(
        "  milled to exile     {:>6.1}      {:>6.1}",
        avg(&all_stats, |s| s.a_milled_to_exile as f64),
        avg(&all_stats, |s| s.b_milled_to_exile as f64)
    );
    println!(
        "  final board size    {:>6.1}      {:>6.1}",
        avg(&all_stats, |s| s.a_final_board as f64),
        avg(&all_stats, |s| s.b_final_board as f64)
    );
    println!(
        "  final graveyard     {:>6.1}      {:>6.1}",
        avg(&all_stats, |s| s.a_final_gy as f64),
        avg(&all_stats, |s| s.b_final_gy as f64)
    );

    println!();
    println!("Event firing breakdown (per-game averages):");
    println!("                          A         B    wired");
    for ev in tsot::card::EventName::ALL {
        let a_avg = avg(&all_stats, |s| {
            s.event_fires.get(&ev).map(|v| v[0]).unwrap_or(0) as f64
        });
        let b_avg = avg(&all_stats, |s| {
            s.event_fires.get(&ev).map(|v| v[1]).unwrap_or(0) as f64
        });
        let any_fired = all_stats
            .iter()
            .any(|s| s.event_fires.get(&ev).is_some_and(|v| v[0] + v[1] > 0));
        let marker = if any_fired { "yes" } else { " no" };
        println!("  {:20} {:>6.2}    {:>6.2}    {}", ev.lua_key(), a_avg, b_avg, marker);
    }

    println!();
    println!("Engine + handler actions (totals across {} games):", all_stats.len());
    println!("                              A         B");
    for action in [
        "draw", "mill", "damage", "move", "discard", "tap", "untap",
        "add_status", "add_modifier", "choose_card", "choose_player",
        "choose_int", "confirm", "activate",
    ] {
        let a_total: u64 = all_stats
            .iter()
            .map(|s| s.action_counts.get(action).map(|v| v[0]).unwrap_or(0) as u64)
            .sum();
        let b_total: u64 = all_stats
            .iter()
            .map(|s| s.action_counts.get(action).map(|v| v[1]).unwrap_or(0) as u64)
            .sum();
        println!("  game.{action:24} {a_total:>6}    {b_total:>6}");
    }

    println!();
    println!("Future-simulation telemetry (per-game averages):");
    println!("                          A         B");
    println!(
        "  preview attempts      {:>6.2}    {:>6.2}",
        avg(&all_stats, |s| s.a_preview_attempts as f64),
        avg(&all_stats, |s| s.b_preview_attempts as f64)
    );
    println!(
        "  rolled back           {:>6.2}    {:>6.2}",
        avg(&all_stats, |s| s.a_preview_rollbacks as f64),
        avg(&all_stats, |s| s.b_preview_rollbacks as f64)
    );
    println!(
        "  mutations explored    {:>6.1}    {:>6.1}",
        avg(&all_stats, |s| s.a_preview_journal_size_total as f64),
        avg(&all_stats, |s| s.b_preview_journal_size_total as f64)
    );

    println!();
    let replay_avg = avg(&all_stats, |s| s.replay_journal_entries as f64);
    let replay_min = all_stats
        .iter()
        .map(|s| s.replay_journal_entries)
        .min()
        .unwrap_or(0);
    let replay_max = all_stats
        .iter()
        .map(|s| s.replay_journal_entries)
        .max()
        .unwrap_or(0);
    println!(
        "Replay journal entries per game:  avg {replay_avg:.1}  min {replay_min}  max {replay_max}"
    );

    println!();
    println!("Pending mechanics (zero where the engine piece hasn't landed):");
    println!("                                  A         B");
    for (label, action) in [
        ("sacrifices (cost P.16)", "sacrificed_as_cost"),
        ("instant responses (R.1)", "instant_response_played"),
        ("artifacts played (P.19)", "artifact_played"),
        ("jewel-tap substitutions (P.24)", "jewel_tap_substitution"),
    ] {
        let a_avg = avg(&all_stats, |s| {
            s.action_counts
                .get(action)
                .map(|v| v[0] as f64)
                .unwrap_or(0.0)
        });
        let b_avg = avg(&all_stats, |s| {
            s.action_counts
                .get(action)
                .map(|v| v[1] as f64)
                .unwrap_or(0.0)
        });
        println!("  {label:32} {a_avg:>6.2}    {b_avg:>6.2}");
    }

    let mut card_play_totals: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let mut card_first_turn_sum: std::collections::BTreeMap<String, (u32, u32)> =
        std::collections::BTreeMap::new();
    for s in &all_stats {
        for (cid, (a_turn, b_turn)) in &s.card_play_turns {
            *card_play_totals.entry(cid.clone()).or_insert(0) += 1;
            for turn in [*a_turn, *b_turn] {
                if turn > 0 {
                    let entry = card_first_turn_sum.entry(cid.clone()).or_insert((0, 0));
                    entry.0 += turn;
                    entry.1 += 1;
                }
            }
        }
    }
    let mut card_rows: Vec<(String, u32, f64)> = card_play_totals
        .iter()
        .map(|(cid, count)| {
            let (sum, n) = card_first_turn_sum.get(cid).copied().unwrap_or((0, 0));
            let mean_turn = if n > 0 { sum as f64 / n as f64 } else { 0.0 };
            (cid.clone(), *count, mean_turn)
        })
        .collect();
    card_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!();
    println!("Top cards by play frequency (across {} games):", all_stats.len());
    println!("  {:<35} {:>10}  {:>10}", "card_id", "games", "mean turn");
    for (cid, count, mean_turn) in card_rows.iter().take(20) {
        let pct = 100.0 * (*count as f64) / (all_stats.len() as f64);
        println!(
            "  {:<35} {:>5}  ({:>3.0}%)  {:>10.1}",
            cid, count, pct, mean_turn
        );
    }

    if !all_stats.is_empty() {
        let mut by_turns: Vec<usize> = (0..all_stats.len()).collect();
        by_turns.sort_by_key(|i| all_stats[*i].turns);
        let shortest = by_turns[0];
        let longest = *by_turns.last().unwrap();
        let mut by_mill: Vec<usize> = (0..all_stats.len()).collect();
        by_mill.sort_by_key(|i| {
            let s = &all_stats[*i];
            std::cmp::Reverse(
                (s.a_milled_to_exile as i64 - s.b_milled_to_exile as i64).abs(),
            )
        });
        let rout = by_mill[0];
        println!();
        println!("Interesting games:");
        for (label, idx) in [
            ("shortest", shortest),
            ("longest ", longest),
            ("rout    ", rout),
        ] {
            let s = &all_stats[idx];
            let (i, j) = game_keys[idx];
            println!(
                "  {label}  turns={:>2}  winner={:?}  {} (A) vs {} (B)  milled A/B = {}/{}",
                s.turns, s.winner, labels[i], labels[j],
                s.a_milled_to_exile, s.b_milled_to_exile
            );
        }
    }

    println!();
    println!("Win-rate matrix (rows = side A, cols = side B; cell = A's win-rate):");
    print!("{:>w$} ", "", w = label_w);
    for j in 0..n {
        print!("{:>9}", format!("[{j}]"));
    }
    print!("{:>9}", "row avg");
    println!();
    for (i, row) in wins.iter().enumerate().take(n) {
        print!("{:>w$} ", labels[i], w = label_w);
        let mut row_sum = 0.0;
        for &cell in row.iter().take(n) {
            let rate = cell as f64 / args.games as f64;
            row_sum += rate;
            print!("{:>9.2}", rate);
        }
        let row_avg = row_sum / n as f64;
        print!("{:>9.2}", row_avg);
        println!();
    }

    println!();
    println!("Per-deck overall (both seats, all opponents):");
    println!(
        "  {:<w$}  {:>10}  {:>10}",
        "deck", "as A", "as B",
        w = label_w
    );
    for (i, label) in labels.iter().enumerate().take(n) {
        let mut as_a_wins = 0u32;
        let mut as_a_games = 0u32;
        let mut as_b_wins = 0u32;
        let mut as_b_games = 0u32;
        #[allow(clippy::needless_range_loop)]
        for j in 0..n {
            if i == j {
                continue;
            }
            as_a_wins += wins[i][j];
            as_a_games += args.games;
            as_b_wins += args.games - wins[j][i];
            as_b_games += args.games;
        }
        let r_a = as_a_wins as f64 / as_a_games as f64;
        let r_b = as_b_wins as f64 / as_b_games as f64;
        println!(
            "  {:<w$}  {:>10.2}  {:>10.2}",
            label, r_a, r_b,
            w = label_w
        );
    }

    println!();
    println!("Elapsed: {:.2?}", elapsed);

    let html_path = &args.html;
    match write_matchup_evolved_html(
        &labels,
        &wins,
        args.games,
        &args.dir,
        html_path,
        &all_stats,
        &game_keys,
        registry.cards(),
    ) {
        Ok(()) => println!("HTML grid written to {html_path}"),
        Err(e) => eprintln!("failed to write HTML to {html_path}: {e}"),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_matchup_evolved_html(
    labels: &[String],
    wins: &[Vec<u32>],
    games: u32,
    dir: &str,
    path: &str,
    all_stats: &[sim::GameStats],
    game_keys: &[(usize, usize)],
    pool: &[tsot::card::Card],
) -> std::io::Result<()> {
    let n = labels.len();
    fn rate_color(r: f64) -> String {
        let t = r.clamp(0.0, 1.0);
        let red = ((1.0 - t) * 100.0 + 30.0) as u8;
        let green = (t * 100.0 + 30.0) as u8;
        format!("background: rgb({red},{green},40); color: #eee;")
    }

    fn avg(stats: &[sim::GameStats], f: impl Fn(&sim::GameStats) -> f64) -> f64 {
        if stats.is_empty() {
            0.0
        } else {
            stats.iter().map(f).sum::<f64>() / stats.len() as f64
        }
    }
    let mut turn_values: Vec<u32> = all_stats.iter().map(|s| s.turns).collect();
    turn_values.sort_unstable();
    let turn_min = turn_values.first().copied().unwrap_or(0);
    let turn_max = turn_values.last().copied().unwrap_or(0);
    let turn_mean = if turn_values.is_empty() {
        0.0
    } else {
        turn_values.iter().sum::<u32>() as f64 / turn_values.len() as f64
    };
    let turn_median = if turn_values.is_empty() {
        0
    } else {
        turn_values[turn_values.len() / 2]
    };

    let per_game_rows: Vec<(&str, f64, f64)> = vec![
        ("cards played", avg(all_stats, |s| s.a_played as f64), avg(all_stats, |s| s.b_played as f64)),
        ("attacks declared", avg(all_stats, |s| s.a_attacks as f64), avg(all_stats, |s| s.b_attacks as f64)),
        ("deaths (own)", avg(all_stats, |s| s.a_deaths as f64), avg(all_stats, |s| s.b_deaths as f64)),
        ("milled to exile", avg(all_stats, |s| s.a_milled_to_exile as f64), avg(all_stats, |s| s.b_milled_to_exile as f64)),
        ("final board size", avg(all_stats, |s| s.a_final_board as f64), avg(all_stats, |s| s.b_final_board as f64)),
        ("final graveyard", avg(all_stats, |s| s.a_final_gy as f64), avg(all_stats, |s| s.b_final_gy as f64)),
    ];

    let event_rows: Vec<(String, f64, f64, bool)> = tsot::card::EventName::ALL
        .iter()
        .map(|ev| {
            let a = avg(all_stats, |s| s.event_fires.get(ev).map(|v| v[0]).unwrap_or(0) as f64);
            let b = avg(all_stats, |s| s.event_fires.get(ev).map(|v| v[1]).unwrap_or(0) as f64);
            let any = all_stats
                .iter()
                .any(|s| s.event_fires.get(ev).is_some_and(|v| v[0] + v[1] > 0));
            (ev.lua_key().to_string(), a, b, any)
        })
        .collect();

    let per_deck_turns: Vec<(f64, u32, u32, u32, u32)> = (0..labels.len())
        .map(|k| {
            let mut ts: Vec<u32> = game_keys
                .iter()
                .enumerate()
                .filter(|(_, (i, j))| *i == k || *j == k)
                .map(|(idx, _)| all_stats[idx].turns)
                .collect();
            if ts.is_empty() {
                (0.0, 0, 0, 0, 0)
            } else {
                ts.sort_unstable();
                let count = ts.len() as u32;
                let mean = ts.iter().sum::<u32>() as f64 / ts.len() as f64;
                let median = ts[ts.len() / 2];
                let mn = *ts.first().unwrap();
                let mx = *ts.last().unwrap();
                (mean, count, mn, median, mx)
            }
        })
        .collect();

    let action_rows: Vec<(String, u64, u64)> = [
        "draw", "mill", "damage", "move", "discard", "tap", "untap",
        "add_status", "add_modifier", "choose_card", "choose_player",
        "choose_int", "confirm", "activate",
    ]
    .iter()
    .map(|action| {
        let a: u64 = all_stats
            .iter()
            .map(|s| s.action_counts.get(*action).map(|v| v[0]).unwrap_or(0) as u64)
            .sum();
        let b: u64 = all_stats
            .iter()
            .map(|s| s.action_counts.get(*action).map(|v| v[1]).unwrap_or(0) as u64)
            .sum();
        (action.to_string(), a, b)
    })
    .collect();

    let future_sim_rows: Vec<(&str, f64, f64)> = vec![
        (
            "preview attempts",
            avg(all_stats, |s| s.a_preview_attempts as f64),
            avg(all_stats, |s| s.b_preview_attempts as f64),
        ),
        (
            "rolled back",
            avg(all_stats, |s| s.a_preview_rollbacks as f64),
            avg(all_stats, |s| s.b_preview_rollbacks as f64),
        ),
        (
            "mutations explored",
            avg(all_stats, |s| s.a_preview_journal_size_total as f64),
            avg(all_stats, |s| s.b_preview_journal_size_total as f64),
        ),
    ];

    let replay_avg = avg(all_stats, |s| s.replay_journal_entries as f64);
    let replay_min = all_stats
        .iter()
        .map(|s| s.replay_journal_entries)
        .min()
        .unwrap_or(0);
    let replay_max = all_stats
        .iter()
        .map(|s| s.replay_journal_entries)
        .max()
        .unwrap_or(0);

    let mut card_play_totals: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let mut card_first_turn_sum: std::collections::BTreeMap<String, (u32, u32)> =
        std::collections::BTreeMap::new();
    for s in all_stats {
        for (cid, (a_turn, b_turn)) in &s.card_play_turns {
            *card_play_totals.entry(cid.clone()).or_insert(0) += 1;
            for turn in [*a_turn, *b_turn] {
                if turn > 0 {
                    let entry = card_first_turn_sum.entry(cid.clone()).or_insert((0, 0));
                    entry.0 += turn;
                    entry.1 += 1;
                }
            }
        }
    }
    let mut card_rows: Vec<(String, u32, f64)> = card_play_totals
        .iter()
        .map(|(cid, count)| {
            let (sum, n) = card_first_turn_sum.get(cid).copied().unwrap_or((0, 0));
            let mean_turn = if n > 0 { sum as f64 / n as f64 } else { 0.0 };
            (cid.clone(), *count, mean_turn)
        })
        .collect();
    card_rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let total_game_count = all_stats.len();

    let interesting: Vec<(&str, usize)> = if all_stats.is_empty() {
        Vec::new()
    } else {
        let mut by_turns: Vec<usize> = (0..all_stats.len()).collect();
        by_turns.sort_by_key(|i| all_stats[*i].turns);
        let shortest = by_turns[0];
        let longest = *by_turns.last().unwrap();
        let mut by_mill: Vec<usize> = (0..all_stats.len()).collect();
        by_mill.sort_by_key(|i| {
            let s = &all_stats[*i];
            std::cmp::Reverse(
                (s.a_milled_to_exile as i64 - s.b_milled_to_exile as i64).abs(),
            )
        });
        let rout = by_mill[0];
        vec![("shortest", shortest), ("longest", longest), ("rout", rout)]
    };

    let pending_rows: Vec<(&str, f64, f64)> = vec![
        ("sacrifices (cost P.16)", "sacrificed_as_cost"),
        ("instant responses (R.1)", "instant_response_played"),
        ("artifacts played (P.19)", "artifact_played"),
        ("jewel-tap substitutions (P.24)", "jewel_tap_substitution"),
    ]
    .into_iter()
    .map(|(label, action)| {
        let a = avg(all_stats, |s| {
            s.action_counts
                .get(action)
                .map(|v| v[0] as f64)
                .unwrap_or(0.0)
        });
        let b = avg(all_stats, |s| {
            s.action_counts
                .get(action)
                .map(|v| v[1] as f64)
                .unwrap_or(0.0)
        });
        (label, a, b)
    })
    .collect();

    let row_rates: Vec<Vec<f64>> = wins
        .iter()
        .map(|row| row.iter().map(|&c| c as f64 / games as f64).collect())
        .collect();
    let row_avgs: Vec<f64> = row_rates
        .iter()
        .map(|row| row.iter().sum::<f64>() / n as f64)
        .collect();
    let mut deck_overall: Vec<(f64, f64)> = Vec::with_capacity(n);
    #[allow(clippy::needless_range_loop)]
    for i in 0..n {
        let mut a_w = 0u32;
        let mut a_g = 0u32;
        let mut b_w = 0u32;
        let mut b_g = 0u32;
        for j in 0..n {
            if i == j {
                continue;
            }
            a_w += wins[i][j];
            a_g += games;
            b_w += games - wins[j][i];
            b_g += games;
        }
        deck_overall.push((a_w as f64 / a_g as f64, b_w as f64 / b_g as f64));
    }

    let markup = html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "tsot — matchup-evolved grid" }
                style { (PreEscaped(report_style::CSS)) }
            }
            body {
                h1 { "tsot — matchup-evolved grid" }
                div.meta {
                    div { span.k { "dir" } b { (dir) } }
                    div { span.k { "decks" } b { (n) } }
                    div { span.k { "games/cell" } b { (games) } }
                    div { span.k { "total games" } b { (n * n * games as usize) } }
                }
                h2 { "Turn count" }
                div.stat-row {
                    div.stat { div.label { "min" } b { (turn_min) } }
                    div.stat { div.label { "median" } b { (turn_median) } }
                    div.stat { div.label { "mean" } b { (format!("{turn_mean:.1}")) } }
                    div.stat { div.label { "max" } b { (turn_max) } }
                }

                h2 { "Per-deck turn count" }
                p.note { "Games where this deck plays either seat. Lower = faster deck." }
                table.summary {
                    thead {
                        tr {
                            th { "deck" }
                            th.num { "games" }
                            th.num { "min" }
                            th.num { "median" }
                            th.num { "mean" }
                            th.num { "max" }
                        }
                    }
                    tbody {
                        @for (k, label) in labels.iter().enumerate() {
                            @let (mean, count, mn, median, mx) = per_deck_turns[k];
                            tr {
                                td { (label) }
                                td.num { (count) }
                                td.num { (mn) }
                                td.num { (median) }
                                td.num { (format!("{mean:.1}")) }
                                td.num { (mx) }
                            }
                        }
                    }
                }

                h2 { "Per-game averages" }
                table.summary {
                    thead { tr { th { "metric" } th.num { "A" } th.num { "B" } } }
                    tbody {
                        @for (label, a, b) in &per_game_rows {
                            tr { td { (label) } td.num { (format!("{a:.1}")) } td.num { (format!("{b:.1}")) } }
                        }
                    }
                }

                h2 { "Event firing breakdown" }
                p.note { "Per-game averages; " em { "wired" } " = 'yes' if any game fired this event." }
                table.summary {
                    thead { tr { th { "event" } th.num { "A" } th.num { "B" } th { "wired" } } }
                    tbody {
                        @for (name, a, b, wired) in &event_rows {
                            tr {
                                td { (name) }
                                td.num { (format!("{a:.2}")) }
                                td.num { (format!("{b:.2}")) }
                                td { @if *wired { "yes" } @else { span.muted { "no" } } }
                            }
                        }
                    }
                }

                h2 { "Engine + handler action totals" }
                p.note { "Totals across all " em { (all_stats.len()) } " games." }
                table.summary {
                    thead { tr { th { "action" } th.num { "A" } th.num { "B" } } }
                    tbody {
                        @for (name, a, b) in &action_rows {
                            tr {
                                td { (format!("game.{name}")) }
                                td.num { (a) }
                                td.num { (b) }
                            }
                        }
                    }
                }

                h2 { "Future-simulation telemetry" }
                p.note { "Per-game averages — every play opens a journal that the AI may roll back." }
                table.summary {
                    thead { tr { th { "metric" } th.num { "A" } th.num { "B" } } }
                    tbody {
                        @for (label, a, b) in &future_sim_rows {
                            tr { td { (label) } td.num { (format!("{a:.2}")) } td.num { (format!("{b:.2}")) } }
                        }
                    }
                }

                h2 { "Replay journal" }
                div.stat-row {
                    div.stat { div.label { "avg entries / game" } b { (format!("{replay_avg:.1}")) } }
                    div.stat { div.label { "min" } b { (replay_min) } }
                    div.stat { div.label { "max" } b { (replay_max) } }
                }

                h2 { "Pending mechanics" }
                p.note { "Per-game averages. Zero indicates the engine piece hasn't landed (or the cards aren't being played)." }
                table.summary {
                    thead { tr { th { "mechanic" } th.num { "A" } th.num { "B" } } }
                    tbody {
                        @for (label, a, b) in &pending_rows {
                            tr { td { (label) } td.num { (format!("{a:.2}")) } td.num { (format!("{b:.2}")) } }
                        }
                    }
                }

                h2 { "Top cards by play frequency" }
                p.note { "Across all " em { (total_game_count) } " games (both sides combined). " em { "mean turn" } " is the average turn this card is first played when it appears." }
                table.summary {
                    thead {
                        tr {
                            th { "card id" }
                            th.num { "games" }
                            th.num { "%" }
                            th.num { "mean turn" }
                        }
                    }
                    tbody {
                        @for (cid, count, mean_turn) in card_rows.iter().take(30) {
                            @let pct = 100.0 * (*count as f64) / (total_game_count as f64);
                            tr {
                                td { (report_style::card_cell(pool, cid)) }
                                td.num { (count) }
                                td.num { (format!("{pct:.0}%")) }
                                td.num { (format!("{mean_turn:.1}")) }
                            }
                        }
                    }
                }

                @if !interesting.is_empty() {
                    h2 { "Interesting games" }
                    p.note { "Three picks from the run: shortest turn count, longest turn count, biggest mill imbalance." }
                    table.summary {
                        thead {
                            tr {
                                th { "category" }
                                th.num { "turns" }
                                th { "winner" }
                                th { "deck A" }
                                th { "deck B" }
                                th.num { "milled A" }
                                th.num { "milled B" }
                            }
                        }
                        tbody {
                            @for (cat, idx) in &interesting {
                                @let s = &all_stats[*idx];
                                @let (i, j) = game_keys[*idx];
                                tr {
                                    td { (cat) }
                                    td.num { (s.turns) }
                                    td { (format!("{:?}", s.winner)) }
                                    td { (labels[i]) }
                                    td { (labels[j]) }
                                    td.num { (s.a_milled_to_exile) }
                                    td.num { (s.b_milled_to_exile) }
                                }
                            }
                        }
                    }
                }

                h2 { "Win-rate matrix" }
                p.note {
                    "Rows = side A, columns = side B. Cell value = A's win-rate over " em { (games) } " games of (row, column). Heat shows wins (green) vs losses (red)."
                }
                table.summary.matchup {
                    thead {
                        tr {
                            th { "" }
                            @for (j, label) in labels.iter().enumerate().take(n) {
                                th { (format!("[{j}] {label}")) }
                            }
                            th { "row avg" }
                        }
                    }
                    tbody {
                        @for (i, row) in row_rates.iter().enumerate().take(n) {
                            tr {
                                th { (format!("[{i}] {}", labels[i])) }
                                @for &rate in row.iter().take(n) {
                                    td.num style=(rate_color(rate)) { (format!("{rate:.2}")) }
                                }
                                td.num { (format!("{:.2}", row_avgs[i])) }
                            }
                        }
                    }
                }
                h2 { "Per-deck overall" }
                p.note { "Win-rate across all opponents (excluding self-matchup)." }
                table.summary {
                    thead {
                        tr {
                            th { "deck" }
                            th.num { "as A" }
                            th.num { "as B" }
                        }
                    }
                    tbody {
                        @for (i, label) in labels.iter().enumerate().take(n) {
                            @let (r_a, r_b) = deck_overall[i];
                            tr {
                                td { (label) }
                                td.num style=(rate_color(r_a)) { (format!("{r_a:.2}")) }
                                td.num style=(rate_color(r_b)) { (format!("{r_b:.2}")) }
                            }
                        }
                    }
                }
            }
        }
    };
    std::fs::write(path, markup.into_string())
}
