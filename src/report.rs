//! HTML report writer for sim runs. Self-contained (inline CSS, no external
//! resources), opens in any browser. Style: dark, monospace, info-dense per
//! QNTX design-philosophy.md; bars/heatmap visual idiom from levi/pnui.
//! Templating: maud (compile-time, auto-escaping).
//!
//! Default path: `tsot-report.html` in cwd. Override with `TSOT_REPORT_OUT`.
//! Set `TSOT_REPORT_OUT=-` to skip.

#![allow(clippy::type_complexity, clippy::manual_checked_ops, clippy::manual_div_ceil)]

use crate::{variant_label, DeckVariant, GameStats, VARIANTS};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use std::time::Duration;
use tsot::{EventName, PlayerId};

pub(crate) fn write_html_report(
    all: &[GameStats],
    pools: &[(DeckVariant, Vec<tsot::Card>)],
    seed: u64,
    elapsed: Duration,
    path: &str,
) -> std::io::Result<()> {
    let markup = build_report(all, pools, seed, elapsed);
    std::fs::write(path, markup.into_string())
}

fn build_report(
    all: &[GameStats],
    pools: &[(DeckVariant, Vec<tsot::Card>)],
    seed: u64,
    elapsed: Duration,
) -> Markup {
    let n = all.len();
    let nf = n.max(1) as f64;
    let a_wins = all.iter().filter(|s| s.winner == PlayerId::A).count();
    let b_wins = n - a_wins;
    let per_game = elapsed / n.max(1) as u32;

    let mut turn_values: Vec<u32> = all.iter().map(|s| s.turns).collect();
    turn_values.sort_unstable();
    let turn_min = turn_values.first().copied().unwrap_or(0);
    let turn_max = turn_values.last().copied().unwrap_or(0);
    let turn_mean = turn_values.iter().sum::<u32>() as f64 / nf;
    let turn_median = turn_values[turn_values.len().saturating_sub(1) / 2];

    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "tsot sim report" }
                style { (PreEscaped(CSS)) }
            }
            body {
                h1 { "tsot sim report" }
                div.meta {
                    div { span.k { "seed" } b { (seed) } }
                    div { span.k { "games" } b { (n) } }
                    div { span.k { "elapsed" } b { (format!("{:.2?}", elapsed)) } }
                    div { span.k { "per game" } b { (format!("{:.1?}", per_game)) } }
                }

                h2 { "overall" }
                div.panel { (win_split("winners", a_wins, b_wins)) }

                h2 { "turn count" }
                div.panel {
                    div.stat-row {
                        (stat("min", turn_min))
                        (stat("median", turn_median))
                        (stat_f("mean", turn_mean))
                        (stat("max", turn_max))
                    }
                    (turn_histogram(&turn_values))
                }

                h2 { "variant pools" }
                div.panel {
                    table.summary {
                        thead { tr { th { "variant" } th { "cards in pool" } } }
                        tbody {
                            @for (v, pool) in pools {
                                tr {
                                    th.vlabel { (variant_label(*v)) }
                                    td.num { (pool.len()) }
                                }
                            }
                        }
                    }
                }

                h2 { "matchup matrix" }
                div.note { "Cell = A-side win rate (n = games in that pairing). Color graded red→grey→green." }
                div.panel { (matchup_matrix(all)) }

                h2 { "per-variant aggregate" }
                div.note { "Win rate pooled across all opponents and both seats." }
                div.panel { (per_variant_aggregate(all)) }

                h2 { "per-game averages" }
                div.panel { (per_game_averages(all, nf)) }

                h2 { "event firing breakdown" }
                div.note { "A.1 triggered abilities. Per-game averages." }
                div.panel { (event_breakdown(all)) }

                h2 { "engine + handler actions" }
                div.note { (format!("Totals across {n} games.")) }
                div.panel { (action_totals(all)) }

                h2 { "future-simulation telemetry" }
                div.note { "Every play opens a journal. Per-game averages unless noted." }
                div.panel { (future_sim_telemetry(all)) }

                h2 { "replay journal" }
                div.note { "Per game, every committed mutation from start to game-end." }
                div.panel { (replay_journal_stats(all)) }

                h2 { "pending mechanics" }
                div.note { "Zero today; nonzero once each engine piece lands." }
                div.panel { (pending_mechanics(all)) }
            }
        }
    }
}

// ---------- Section helpers ----------

fn stat<T: std::fmt::Display>(label: &str, value: T) -> Markup {
    html! {
        div.stat {
            div.label { (label) }
            b { (value) }
        }
    }
}

fn stat_f(label: &str, value: f64) -> Markup {
    html! {
        div.stat {
            div.label { (label) }
            b { (format!("{value:.1}")) }
        }
    }
}

fn win_split(label: &str, a: usize, b: usize) -> Markup {
    let total = (a + b).max(1) as f64;
    let a_pct = 100.0 * a as f64 / total;
    let b_pct = 100.0 * b as f64 / total;
    html! {
        div.win-split {
            div.win-label { (label) }
            div.win-bar {
                div.win-a style=(format!("width:{a_pct:.1}%")) {
                    span { (format!("A {a} · {a_pct:.0}%")) }
                }
                div.win-b style=(format!("width:{b_pct:.1}%")) {
                    span { (format!("B {b} · {b_pct:.0}%")) }
                }
            }
        }
    }
}

fn turn_histogram(sorted: &[u32]) -> Markup {
    if sorted.is_empty() {
        return html! {};
    }
    let lo = sorted.first().copied().unwrap_or(0);
    let hi = sorted.last().copied().unwrap_or(0);
    let span = hi.saturating_sub(lo).max(1);
    let n_bins: u32 = 24;
    let bin_width = ((span as f64) / (n_bins as f64)).ceil() as u32;
    let mut bins = vec![0u32; n_bins as usize];
    for &t in sorted {
        let idx = if bin_width == 0 {
            0
        } else {
            ((t - lo) / bin_width).min(n_bins - 1) as usize
        };
        bins[idx] += 1;
    }
    let max_count = *bins.iter().max().unwrap_or(&1).max(&1) as f64;
    html! {
        div.hist {
            @for (i, &count) in bins.iter().enumerate() {
                @let height = 100.0 * count as f64 / max_count;
                @let bin_lo = lo + (i as u32) * bin_width;
                @let bin_hi = bin_lo + bin_width.saturating_sub(1);
                div.hist-bin title=(format!("turns {bin_lo}–{bin_hi}: {count} games")) {
                    div.hist-bar style=(format!("height:{height:.0}%")) {}
                }
            }
        }
        div.hist-axis {
            span { (lo) }
            span { (hi) }
        }
    }
}

fn matchup_matrix(all: &[GameStats]) -> Markup {
    html! {
        table.matchup {
            thead {
                tr {
                    th {}
                    @for v in &VARIANTS {
                        th { "B: " (variant_label(*v)) }
                    }
                }
            }
            tbody {
                @for va in &VARIANTS {
                    tr {
                        th.vlabel { "A: " (variant_label(*va)) }
                        @for vb in &VARIANTS {
                            @let games: Vec<&GameStats> = all.iter()
                                .filter(|s| s.variant_a == *va && s.variant_b == *vb)
                                .collect();
                            @if games.is_empty() {
                                td.empty { "—" }
                            } @else {
                                @let wins = games.iter().filter(|s| s.winner == PlayerId::A).count();
                                @let rate = wins as f64 / games.len() as f64;
                                @let bg = rate_to_color(rate);
                                td style=(format!("background:{bg}")) {
                                    div.rate { (format!("{rate:.2}")) }
                                    div.sub { "n=" (games.len()) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn per_variant_aggregate(all: &[GameStats]) -> Markup {
    html! {
        table.summary {
            thead { tr {
                th { "variant" } th { "games" } th { "wins" } th { "rate" } th {}
            }}
            tbody {
                @for v in &VARIANTS {
                    @let (games, wins) = aggregate_for(*v, all);
                    @let rate = if games > 0 { wins as f64 / games as f64 } else { 0.0 };
                    @let bg = rate_to_color(rate);
                    tr {
                        th.vlabel { (variant_label(*v)) }
                        td.num { (games) }
                        td.num { (wins) }
                        td.num style=(format!("background:{bg}")) { (format!("{rate:.2}")) }
                        td.bar-cell {
                            div.bar {
                                div.bar-fill style=(format!("width:{:.0}%; background:{bg}", rate * 100.0)) {}
                            }
                        }
                    }
                }
            }
        }
    }
}

fn aggregate_for(v: DeckVariant, all: &[GameStats]) -> (u32, u32) {
    let mut games = 0u32;
    let mut wins = 0u32;
    for s in all {
        if s.variant_a == v {
            games += 1;
            if s.winner == PlayerId::A {
                wins += 1;
            }
        }
        if s.variant_b == v {
            games += 1;
            if s.winner == PlayerId::B {
                wins += 1;
            }
        }
    }
    (games, wins)
}

fn per_game_averages(all: &[GameStats], _nf: f64) -> Markup {
    let rows: [(&str, fn(&GameStats) -> (f64, f64)); 6] = [
        ("cards played", |s| (s.a_played as f64, s.b_played as f64)),
        ("attacks declared", |s| (s.a_attacks as f64, s.b_attacks as f64)),
        ("deaths (own creat.)", |s| (s.a_deaths as f64, s.b_deaths as f64)),
        ("milled to exile", |s| (s.a_milled_to_exile as f64, s.b_milled_to_exile as f64)),
        ("final board size", |s| (s.a_final_board as f64, s.b_final_board as f64)),
        ("final graveyard", |s| (s.a_final_gy as f64, s.b_final_gy as f64)),
    ];
    html! {
        div.note { "Per-variant means: pooled across both seats for that variant. Cell colored by intensity relative to the row max." }
        table.summary {
            thead { tr {
                th { "metric" }
                @for v in &VARIANTS { th.vlabel { (variant_label(*v)) } }
            }}
            tbody {
                @for (label, f) in rows {
                    @let per = per_variant_avg_f(all, f);
                    @let row_max = per.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(0.001);
                    tr {
                        th { (label) }
                        @for (_, val) in &per {
                            @let bg = intensity_color(val / row_max);
                            td.num style=(format!("background:{bg}")) { (format!("{val:.1}")) }
                        }
                    }
                }
            }
        }
    }
}

/// Per-variant pooled average: for each variant, average the metric across
/// every game where that variant plays (as A-side using `f(s).0` and as
/// B-side using `f(s).1`). Returns one entry per variant in `VARIANTS` order.
fn per_variant_avg_f<F: Fn(&GameStats) -> (f64, f64)>(
    all: &[GameStats],
    f: F,
) -> Vec<(DeckVariant, f64)> {
    VARIANTS
        .iter()
        .map(|v| {
            let mut total = 0.0;
            let mut count = 0u32;
            for s in all {
                let (a, b) = f(s);
                if s.variant_a == *v {
                    total += a;
                    count += 1;
                }
                if s.variant_b == *v {
                    total += b;
                    count += 1;
                }
            }
            let avg = if count > 0 {
                total / count as f64
            } else {
                0.0
            };
            (*v, avg)
        })
        .collect()
}

/// Per-variant pooled total of a u64 metric (action counts, etc.).
fn per_variant_total_u64<F: Fn(&GameStats) -> (u64, u64)>(
    all: &[GameStats],
    f: F,
) -> Vec<(DeckVariant, u64)> {
    VARIANTS
        .iter()
        .map(|v| {
            let mut total = 0u64;
            for s in all {
                let (a, b) = f(s);
                if s.variant_a == *v {
                    total += a;
                }
                if s.variant_b == *v {
                    total += b;
                }
            }
            (*v, total)
        })
        .collect()
}

/// Sample-size denominator for a variant: how many games it played
/// (counting each side).
fn variant_games(all: &[GameStats], v: DeckVariant) -> u32 {
    let mut n = 0u32;
    for s in all {
        if s.variant_a == v {
            n += 1;
        }
        if s.variant_b == v {
            n += 1;
        }
    }
    n
}

/// Intensity color: 0 → panel bg, 1 → accent green at low alpha.
fn intensity_color(t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    let alpha = (0.04 + 0.32 * t) * 255.0;
    format!("rgba(125, 186, 138, {:.2})", alpha / 255.0)
}

fn event_breakdown(all: &[GameStats]) -> Markup {
    html! {
        table.summary {
            thead { tr {
                th { "event" }
                @for v in &VARIANTS { th.vlabel { (variant_label(*v)) } }
                th { "wired" }
            }}
            tbody {
                @for ev in EventName::ALL {
                    @let per = per_variant_avg_f(all, |s| {
                        let f = s.event_fires.get(&ev);
                        (
                            f.map(|v| v[0]).unwrap_or(0) as f64,
                            f.map(|v| v[1]).unwrap_or(0) as f64,
                        )
                    });
                    @let row_max = per.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(0.001);
                    @let any_fired = all.iter()
                        .any(|s| s.event_fires.get(&ev).is_some_and(|v| v[0] + v[1] > 0));
                    tr {
                        th { (ev.lua_key()) }
                        @for (_, val) in &per {
                            @let bg = intensity_color(val / row_max);
                            td.num style=(format!("background:{bg}")) { (format!("{val:.2}")) }
                        }
                        td {
                            @if any_fired { span.ok { "yes" } }
                            @else { span.muted { "no" } }
                        }
                    }
                }
            }
        }
    }
}

fn action_totals(all: &[GameStats]) -> Markup {
    let actions = [
        "draw", "mill", "damage", "move", "discard", "tap", "untap",
        "add_status", "add_modifier", "choose_card", "choose_player",
        "choose_int", "confirm", "decked_by_handler_draw",
        "preview_skip_suicide", "preview_retry_rescued",
        "counter_top", "counter", "instant_response_played",
    ];
    html! {
        div.note { "Per-variant per-game averages (totals scaled by that variant's game count). Cell color = intensity within the row." }
        table.summary {
            thead { tr {
                th { "action" }
                @for v in &VARIANTS { th.vlabel { (variant_label(*v)) } }
            }}
            tbody {
                @for action in actions {
                    @let totals = per_variant_total_u64(all, |s| {
                        let v = s.action_counts.get(action);
                        (
                            v.map(|x| x[0]).unwrap_or(0) as u64,
                            v.map(|x| x[1]).unwrap_or(0) as u64,
                        )
                    });
                    @let avgs: Vec<(DeckVariant, f64)> = totals.iter()
                        .map(|(v, t)| {
                            let n = variant_games(all, *v) as f64;
                            (*v, if n > 0.0 { *t as f64 / n } else { 0.0 })
                        })
                        .collect();
                    @let row_max = avgs.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(0.001);
                    tr {
                        th { "game." (action) }
                        @for (_, val) in &avgs {
                            @let bg = intensity_color(val / row_max);
                            td.num style=(format!("background:{bg}")) { (format!("{val:.2}")) }
                        }
                    }
                }
            }
        }
    }
}

fn future_sim_telemetry(all: &[GameStats]) -> Markup {
    let rows: [(&str, fn(&GameStats) -> (f64, f64)); 3] = [
        ("preview attempts", |s| (s.a_preview_attempts as f64, s.b_preview_attempts as f64)),
        ("rolled back", |s| (s.a_preview_rollbacks as f64, s.b_preview_rollbacks as f64)),
        ("mutations explored (sum journal/game)", |s| (
            s.a_preview_journal_size_total as f64,
            s.b_preview_journal_size_total as f64,
        )),
    ];
    html! {
        table.summary {
            thead { tr {
                th { "metric" }
                @for v in &VARIANTS { th.vlabel { (variant_label(*v)) } }
            }}
            tbody {
                @for (label, f) in rows {
                    @let per = per_variant_avg_f(all, f);
                    @let row_max = per.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(0.001);
                    tr {
                        th { (label) }
                        @for (_, val) in &per {
                            @let bg = intensity_color(val / row_max);
                            td.num style=(format!("background:{bg}")) { (format!("{val:.2}")) }
                        }
                    }
                }
                @let per_mp = per_variant_avg_f(all, |s| {
                    let a = if s.a_preview_attempts == 0 { 0.0 }
                            else { s.a_preview_journal_size_total as f64 / s.a_preview_attempts as f64 };
                    let b = if s.b_preview_attempts == 0 { 0.0 }
                            else { s.b_preview_journal_size_total as f64 / s.b_preview_attempts as f64 };
                    (a, b)
                });
                @let mp_max = per_mp.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(0.001);
                tr {
                    th { "avg mutations / play" }
                    @for (_, val) in &per_mp {
                        @let bg = intensity_color(val / mp_max);
                        td.num style=(format!("background:{bg}")) { (format!("{val:.2}")) }
                    }
                }
            }
        }
    }
}

fn replay_journal_stats(all: &[GameStats]) -> Markup {
    let replay_avg = avg(all, |s| s.replay_journal_entries as f64);
    let replay_min = all.iter().map(|s| s.replay_journal_entries).min().unwrap_or(0);
    let replay_max = all.iter().map(|s| s.replay_journal_entries).max().unwrap_or(0);
    html! {
        div.stat-row {
            (stat_f("avg", replay_avg))
            (stat("min", replay_min))
            (stat("max", replay_max))
        }
        div.note style="margin-top: 1em" { "Per-variant means (pooled both seats):" }
        table.summary {
            thead { tr {
                th { "variant" }
                th { "avg journal entries" }
            }}
            tbody {
                @let per = per_variant_avg_f(all, |s| {
                    (s.replay_journal_entries as f64, s.replay_journal_entries as f64)
                });
                @let row_max = per.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(0.001);
                @for (v, val) in &per {
                    @let bg = intensity_color(val / row_max);
                    tr {
                        th.vlabel { (variant_label(*v)) }
                        td.num style=(format!("background:{bg}")) { (format!("{val:.1}")) }
                    }
                }
            }
        }
    }
}

fn pending_mechanics(all: &[GameStats]) -> Markup {
    let resp = per_variant_avg_f(all, |s| {
        let v = s.action_counts.get("instant_response_played");
        (
            v.map(|x| x[0] as f64).unwrap_or(0.0),
            v.map(|x| x[1] as f64).unwrap_or(0.0),
        )
    });
    let zero_row: Vec<(DeckVariant, f64)> = VARIANTS.iter().map(|v| (*v, 0.0)).collect();
    let pending: [(&str, &Vec<(DeckVariant, f64)>); 8] = [
        ("sacrifices (cost P.16)", &zero_row),
        ("activated abilities used", &zero_row),
        ("instant responses (R.1)", &resp),
        ("artifacts played (P.19)", &zero_row),
        ("environments played (P.21)", &zero_row),
        ("mulligans (S.2/S.3)", &zero_row),
        ("counters on the stack", &zero_row),
        ("color/symbol/type mutations", &zero_row),
    ];
    html! {
        table.summary {
            thead { tr {
                th { "mechanic" }
                @for v in &VARIANTS { th.vlabel { (variant_label(*v)) } }
            }}
            tbody {
                @for (label, per) in pending {
                    @let all_zero = per.iter().all(|(_, v)| *v == 0.0);
                    @let row_max = per.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(0.001);
                    tr.muted[all_zero] {
                        th { (label) }
                        @for (_, val) in per {
                            @if all_zero {
                                td.num { (format!("{val:.2}")) }
                            } @else {
                                @let bg = intensity_color(val / row_max);
                                td.num style=(format!("background:{bg}")) { (format!("{val:.2}")) }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn avg<F: Fn(&GameStats) -> f64>(all: &[GameStats], f: F) -> f64 {
    all.iter().map(f).sum::<f64>() / all.len().max(1) as f64
}

fn rate_to_color(rate: f64) -> String {
    let r = rate.clamp(0.0, 1.0);
    let (cr, cg, cb) = if r < 0.5 {
        let t = r * 2.0;
        (
            (220.0 * (1.0 - t) + 60.0 * t) as u8,
            (70.0 * (1.0 - t) + 60.0 * t) as u8,
            (70.0 * (1.0 - t) + 60.0 * t) as u8,
        )
    } else {
        let t = (r - 0.5) * 2.0;
        (
            (60.0 * (1.0 - t) + 70.0 * t) as u8,
            (60.0 * (1.0 - t) + 180.0 * t) as u8,
            (60.0 * (1.0 - t) + 100.0 * t) as u8,
        )
    };
    format!("rgb({cr},{cg},{cb})")
}

const CSS: &str = r#"
:root {
  --bg-page: #1a1b1a;
  --bg-panel: #252625;
  --bg-panel-alt: #2e2f2e;
  --bg-row-hover: #2a2b2a;
  --border: #3f4140;
  --text: #dfe1e0;
  --text-secondary: #a9abaa;
  --text-tertiary: #868787;
  --text-emphasis: #fefffe;
  --accent: #7dba8a;
  --accent-dim: #5a9a6a;
  --player-a: #7eb8da;
  --player-b: #d4a87e;
}
* { box-sizing: border-box; }
body {
  font-family: 'JetBrains Mono', 'SF Mono', Monaco, 'Fira Code', Consolas, monospace;
  background: var(--bg-page);
  color: var(--text);
  max-width: 1100px;
  margin: 2em auto;
  padding: 0 1.5em 4em;
  font-size: 13px;
  line-height: 1.45;
}
h1 {
  color: var(--text-emphasis);
  border-bottom: 1px solid var(--border);
  padding-bottom: 0.4em;
  font-size: 22px;
  font-weight: 600;
}
h2 {
  margin-top: 2.2em;
  color: var(--text-emphasis);
  font-size: 14px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 1px;
  opacity: 0.85;
}
.meta {
  display: flex;
  gap: 2em;
  flex-wrap: wrap;
  margin: 1em 0;
  padding: 0.8em 1em;
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 3px;
}
.meta .k {
  color: var(--text-tertiary);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
  margin-right: 0.5em;
}
.meta b { color: var(--accent); font-weight: 600; }
.note {
  color: var(--text-tertiary);
  font-size: 11px;
  margin: 0.4em 0;
}
.panel {
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 3px;
  padding: 1em 1.2em;
  margin: 0.5em 0 1.5em;
}
.stat-row {
  display: flex;
  gap: 2em;
  flex-wrap: wrap;
}
.stat .label {
  color: var(--text-tertiary);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
}
.stat b {
  color: var(--accent);
  font-size: 18px;
  font-weight: 600;
}
table { border-collapse: collapse; width: 100%; }
table th, table td { padding: 4px 10px; text-align: left; }
table thead th {
  color: var(--text-tertiary);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
  font-weight: 600;
  border-bottom: 1px solid var(--border);
  padding-bottom: 6px;
}
table tbody tr:hover { background: var(--bg-row-hover); }
.summary th { color: var(--text-secondary); font-weight: normal; }
.num { text-align: right; font-variant-numeric: tabular-nums; }
.vlabel { color: var(--accent); font-weight: 600; }
.muted { color: var(--text-tertiary); }
.muted .num { color: var(--text-tertiary); }
.ok { color: var(--accent); }
.matchup th, .matchup td {
  text-align: center;
  padding: 8px 10px;
  border: 1px solid var(--border);
  min-width: 64px;
}
.matchup thead th { background: var(--bg-panel-alt); }
.matchup tbody th { background: var(--bg-panel-alt); color: var(--accent); }
.matchup td.empty { color: var(--text-tertiary); background: var(--bg-panel-alt); }
.matchup td .rate { font-size: 13px; font-weight: 600; color: #fff; text-shadow: 0 1px 0 rgba(0,0,0,0.4); }
.matchup td .sub { font-size: 9px; color: rgba(255,255,255,0.7); }
.win-split { margin-bottom: 0.5em; }
.win-label {
  font-size: 10px;
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 1px;
  margin-bottom: 4px;
}
.win-bar {
  display: flex;
  width: 100%;
  height: 24px;
  background: var(--bg-panel-alt);
  border-radius: 2px;
  overflow: hidden;
  border: 1px solid var(--border);
}
.win-a, .win-b {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  color: rgba(0, 0, 0, 0.75);
  font-weight: 600;
}
.win-a { background: var(--player-a); }
.win-b { background: var(--player-b); }
.bar {
  width: 100%;
  max-width: 200px;
  height: 8px;
  background: var(--bg-panel-alt);
  overflow: hidden;
}
.bar-fill { height: 100%; transition: width 0.25s ease; }
.bar-cell { width: 220px; }
.ab-bar-pair {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 200px;
}
.ab-bar-pair.small { width: 160px; }
.ab-bar {
  height: 6px;
  background: var(--bg-panel-alt);
  overflow: hidden;
}
.ab-bar-fill { height: 100%; opacity: 0.85; transition: width 0.25s ease; }
.ab-bar-fill.a { background: var(--player-a); }
.ab-bar-fill.b { background: var(--player-b); }
.ab-bar-cell { width: 220px; }
.hist {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 80px;
  margin-top: 1em;
  padding: 0 2px;
}
.hist-bin {
  flex: 1;
  height: 100%;
  display: flex;
  align-items: flex-end;
  cursor: pointer;
}
.hist-bar {
  width: 100%;
  background: var(--accent-dim);
  transition: background 0.15s;
  min-height: 1px;
}
.hist-bin:hover .hist-bar { background: var(--accent); }
.hist-axis {
  display: flex;
  justify-content: space-between;
  font-size: 9px;
  color: var(--text-tertiary);
  margin-top: 4px;
  padding: 0 2px;
}
"#;
