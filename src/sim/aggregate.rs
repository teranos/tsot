//! Terminal-format aggregator. Prints the matchup matrix, per-variant
//! aggregate, action totals, etc. to stdout after the matchup run
//! finishes. Reads from a slice of [`super::stats::GameStats`].

use tsot::card::EventName;
use tsot::game::PlayerId;

use super::stats::GameStats;
use super::variants::{variant_label, VARIANTS};

pub fn print_aggregate(all: &[GameStats], elapsed: std::time::Duration) {
    let n = all.len() as f64;
    let a_wins = all.iter().filter(|s| s.winner == PlayerId::A).count();
    let b_wins = all.iter().filter(|s| s.winner == PlayerId::B).count();

    let mut turn_values: Vec<u32> = all.iter().map(|s| s.turns).collect();
    turn_values.sort_unstable();
    let turn_min = turn_values.first().copied().unwrap_or(0);
    let turn_max = turn_values.last().copied().unwrap_or(0);
    let turn_mean: f64 = turn_values.iter().sum::<u32>() as f64 / n;
    let turn_median = turn_values[turn_values.len() / 2];

    println!();
    println!(
        "=== Aggregate over {} games (elapsed {:.2?}, avg {:.1?} per game) ===",
        all.len(),
        elapsed,
        elapsed / all.len() as u32
    );
    println!();
    println!(
        "Winners:  A {} ({:.0}%)   B {} ({:.0}%)",
        a_wins,
        100.0 * a_wins as f64 / n,
        b_wins,
        100.0 * b_wins as f64 / n,
    );
    println!();
    println!(
        "Turn count:  min {}   median {}   mean {:.1}   max {}",
        turn_min, turn_median, turn_mean, turn_max
    );
    println!();
    println!("Per-game averages:");
    println!("                       A           B");
    println!(
        "  cards played        {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_played as f64),
        avg(all, |s| s.b_played as f64)
    );
    println!(
        "  attacks declared    {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_attacks as f64),
        avg(all, |s| s.b_attacks as f64)
    );
    println!(
        "  deaths (own creat.) {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_deaths as f64),
        avg(all, |s| s.b_deaths as f64)
    );
    println!(
        "  milled to exile     {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_milled_to_exile as f64),
        avg(all, |s| s.b_milled_to_exile as f64)
    );
    println!(
        "  final board size    {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_final_board as f64),
        avg(all, |s| s.b_final_board as f64)
    );
    println!(
        "  final graveyard     {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_final_gy as f64),
        avg(all, |s| s.b_final_gy as f64)
    );

    println!();
    println!("Event firing breakdown (per-game averages, A.1 triggered abilities):");
    println!("                          A         B    wired");
    for ev in EventName::ALL {
        let a_avg = avg(all, |s| {
            s.event_fires.get(&ev).map(|v| v[0]).unwrap_or(0) as f64
        });
        let b_avg = avg(all, |s| {
            s.event_fires.get(&ev).map(|v| v[1]).unwrap_or(0) as f64
        });
        let any_fired = all
            .iter()
            .any(|s| s.event_fires.get(&ev).is_some_and(|v| v[0] + v[1] > 0));
        let marker = if any_fired { "yes" } else { " no" };
        println!("  {:20} {:>6.2}    {:>6.2}    {}", ev.lua_key(), a_avg, b_avg, marker);
    }

    println!();
    println!("Engine + handler actions (totals across {} games):", all.len());
    println!("                              A         B");
    for action in [
        "draw",
        "mill",
        "damage",
        "move",
        "discard",
        "tap",
        "untap",
        "add_status",
        "add_modifier",
        "choose_card",
        "choose_player",
        "choose_int",
        "confirm",
        "decked_by_handler_draw",
        "preview_skip_suicide",
        "preview_retry_rescued",
        "counter_top",
        "instant_response_played",
        "artifact_played",
        "jewel_tap_substitution",
    ] {
        let a_total: u64 = all
            .iter()
            .map(|s| s.action_counts.get(action).map(|v| v[0]).unwrap_or(0) as u64)
            .sum();
        let b_total: u64 = all
            .iter()
            .map(|s| s.action_counts.get(action).map(|v| v[1]).unwrap_or(0) as u64)
            .sum();
        println!("  game.{action:24} {a_total:>6}    {b_total:>6}");
    }

    println!();
    println!("Future-simulation telemetry (per-game averages — every play opens a journal):");
    println!("                          A         B");
    let attempts_a = avg(all, |s| s.a_preview_attempts as f64);
    let attempts_b = avg(all, |s| s.b_preview_attempts as f64);
    println!("  preview attempts      {attempts_a:>6.2}    {attempts_b:>6.2}");
    println!(
        "  rolled back           {:>6.2}    {:>6.2}",
        avg(all, |s| s.a_preview_rollbacks as f64),
        avg(all, |s| s.b_preview_rollbacks as f64)
    );
    println!(
        "  mutations explored    {:>6.1}    {:>6.1}    (sum of journal entries per game)",
        avg(all, |s| s.a_preview_journal_size_total as f64),
        avg(all, |s| s.b_preview_journal_size_total as f64)
    );
    let avg_size_a = if attempts_a > 0.0 {
        avg(all, |s| s.a_preview_journal_size_total as f64) / attempts_a
    } else {
        0.0
    };
    let avg_size_b = if attempts_b > 0.0 {
        avg(all, |s| s.b_preview_journal_size_total as f64) / attempts_b
    } else {
        0.0
    };
    println!(
        "  avg mutations / play  {avg_size_a:>6.2}    {avg_size_b:>6.2}    (depth of each previewed future)"
    );
    let replay_avg = avg(all, |s| s.replay_journal_entries as f64);
    let replay_min = all
        .iter()
        .map(|s| s.replay_journal_entries)
        .min()
        .unwrap_or(0);
    let replay_max = all
        .iter()
        .map(|s| s.replay_journal_entries)
        .max()
        .unwrap_or(0);
    println!();
    println!(
        "Replay journal (per game, captures every committed mutation from start to game-end):"
    );
    println!(
        "  entries   avg {replay_avg:>6.1}   min {replay_min:>4}   max {replay_max:>4}"
    );

    println!();
    println!("Pending mechanics (zero today; nonzero once each engine piece lands):");
    println!("                                  A         B");
    let sac_a = avg(all, |s| {
        s.action_counts
            .get("sacrificed_as_cost")
            .map(|v| v[0] as f64)
            .unwrap_or(0.0)
    });
    let sac_b = avg(all, |s| {
        s.action_counts
            .get("sacrificed_as_cost")
            .map(|v| v[1] as f64)
            .unwrap_or(0.0)
    });
    println!("  {:35} {:>6.2}    {:>6.2}", "sacrifices (cost P.16)", sac_a, sac_b);
    print_pending("activated abilities used");
    let resp_a = avg(all, |s| {
        s.action_counts
            .get("instant_response_played")
            .map(|v| v[0] as f64)
            .unwrap_or(0.0)
    });
    let resp_b = avg(all, |s| {
        s.action_counts
            .get("instant_response_played")
            .map(|v| v[1] as f64)
            .unwrap_or(0.0)
    });
    println!("  {:35} {:>6.2}    {:>6.2}", "instant responses (R.1)", resp_a, resp_b);
    let arts_a = avg(all, |s| {
        s.action_counts
            .get("artifact_played")
            .map(|v| v[0] as f64)
            .unwrap_or(0.0)
    });
    let arts_b = avg(all, |s| {
        s.action_counts
            .get("artifact_played")
            .map(|v| v[1] as f64)
            .unwrap_or(0.0)
    });
    println!("  {:35} {:>6.2}    {:>6.2}", "artifacts played (P.19)", arts_a, arts_b);
    print_pending("environments played (P.21)");
    print_pending("mulligans (S.2/S.3)");
    print_pending("counters on the stack");
    print_pending("color/symbol/type mutations");

    println!();
    println!("Matchup matrix (cell = A-side win rate; n = games in that pairing):");
    print!("           ");
    for v in &VARIANTS {
        print!("  B:{}    ", variant_label(*v));
    }
    println!();
    for va in &VARIANTS {
        print!("  A:{}     ", variant_label(*va));
        for vb in &VARIANTS {
            let games: Vec<&GameStats> = all
                .iter()
                .filter(|s| s.variant_a == *va && s.variant_b == *vb)
                .collect();
            if games.is_empty() {
                print!("  --  ({:>2})", 0);
                continue;
            }
            let wins = games.iter().filter(|s| s.winner == PlayerId::A).count();
            let rate = wins as f64 / games.len() as f64;
            print!(" {:>4.2} ({:>3})", rate, games.len());
        }
        println!();
    }

    println!();
    if let Some(first) = all.first() {
        if first.token_a.len() == 16 {
            println!(
                "Master seed signature: {}    (trailing 12 chars of every token in this run)",
                &first.token_a[4..16]
            );
        }
    }
    println!("Interesting games (deck tokens — short = first 4 chars of full 16-char token):");
    print_interesting_games(all);
    println!();
    println!("Per-variant aggregate win rate (across all opponents, both sides):");
    println!("  Variant   games   wins   rate");
    for v in &VARIANTS {
        let mut games = 0u32;
        let mut wins = 0u32;
        for s in all {
            if s.variant_a == *v {
                games += 1;
                if s.winner == PlayerId::A {
                    wins += 1;
                }
            }
            if s.variant_b == *v {
                games += 1;
                if s.winner == PlayerId::B {
                    wins += 1;
                }
            }
        }
        let rate = if games > 0 {
            wins as f64 / games as f64
        } else {
            0.0
        };
        println!(
            "  {}        {:>5}   {:>4}   {:.2}",
            variant_label(*v),
            games,
            wins,
            rate
        );
    }
}

fn print_pending(label: &str) {
    println!("  {label:35} {:>6.1}    {:>6.1}", 0.0_f64, 0.0_f64);
}

/// Pick a handful of "interesting" games and print their deck tokens so
/// you can replay them via TSOT_DECK_A_TOKEN / TSOT_DECK_B_TOKEN. Picks
/// by three criteria: shortest game (decisive opening), longest game
/// (close race), and biggest mill-imbalance (one-sided rout).
fn print_interesting_games(all: &[GameStats]) {
    if all.is_empty() {
        return;
    }
    // Shortest by turn count.
    let mut by_turns = all.iter().collect::<Vec<_>>();
    by_turns.sort_by_key(|s| s.turns);
    print_one_game("shortest", by_turns[0]);
    // Longest by turn count.
    print_one_game("longest ", by_turns[by_turns.len() - 1]);
    // Biggest mill imbalance: absolute difference between defenders milled.
    let mut by_mill = all.iter().collect::<Vec<_>>();
    by_mill.sort_by_key(|s| {
        std::cmp::Reverse(
            (s.a_milled_to_exile as i64 - s.b_milled_to_exile as i64).abs(),
        )
    });
    print_one_game("rout    ", by_mill[0]);
}

fn print_one_game(label: &str, s: &GameStats) {
    // Show the 4-char short forms (per-game tuple bits) — the master
    // seed signature is shared across all games in the run and is
    // already printed once at the top. Full 16-char tokens stay in
    // GameStats and surface in the HTML report tooltips.
    let short_a = if s.token_a.len() >= 4 {
        &s.token_a[0..4]
    } else {
        s.token_a.as_str()
    };
    let short_b = if s.token_b.len() >= 4 {
        &s.token_b[0..4]
    } else {
        s.token_b.as_str()
    };
    println!(
        "  {label}  turns={:>2}  winner={:?}  {}/{}/g{}  A={} B={}",
        s.turns,
        s.winner,
        variant_label(s.variant_a),
        variant_label(s.variant_b),
        s.game_index,
        short_a,
        short_b,
    );
}

fn avg<F: Fn(&GameStats) -> f64>(all: &[GameStats], f: F) -> f64 {
    all.iter().map(f).sum::<f64>() / all.len() as f64
}
