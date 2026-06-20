//! HTML report writer for `tsot champions-report`. Reuses the dark-mono
//! style from `report.rs` via the shared `CSS` constant. Self-contained
//! (inline CSS, no external resources) — opens in any browser.

use std::collections::{BTreeMap, BTreeSet};

use maud::{html, Markup, PreEscaped, DOCTYPE};
use tsot::Card;

use crate::report_style::{self, CSS};
use tsot::sim::evolved_deck::EvolvedDeck;
use crate::cli_champions_report::ChampGameStats;

pub(crate) fn write_html_report(
    champions: &[EvolvedDeck],
    playable_pool: &[Card],
    dir: &str,
    path: &str,
    game_stats: &[ChampGameStats],
) -> std::io::Result<()> {
    let markup = build(champions, playable_pool, dir, game_stats);
    std::fs::write(path, markup.into_string())
}

struct GameRow {
    label: String,
    count: usize,
    min: u32,
    median: u32,
    mean: f64,
    max: u32,
    attacks_avg: f64,
    milled_avg: f64,
}

fn build(
    champions: &[EvolvedDeck],
    playable_pool: &[Card],
    dir: &str,
    game_stats: &[ChampGameStats],
) -> Markup {
    // Pre-compute per-champion game-row aggregates so the maud template
    // stays declarative (no statement blocks).
    let game_rows: Vec<GameRow> = champions
        .iter()
        .zip(game_stats.iter())
        .filter(|(_, gs)| !gs.turns.is_empty())
        .map(|(c, gs)| {
            let mut ts = gs.turns.clone();
            ts.sort_unstable();
            let count = ts.len();
            let mean = ts.iter().sum::<u32>() as f64 / count as f64;
            let attacks_avg = gs.attacks as f64 / count as f64;
            let milled_avg = gs.milled as f64 / count as f64;
            GameRow {
                label: c.label.clone(),
                count,
                min: *ts.first().unwrap(),
                median: ts[ts.len() / 2],
                mean,
                max: *ts.last().unwrap(),
                attacks_avg,
                milled_avg,
            }
        })
        .collect();
    let n = champions.len();
    let fits: Vec<f64> = champions.iter().map(|c| c.fitness).collect();
    let fit_mean: f64 = if n > 0 {
        fits.iter().sum::<f64>() / (n as f64)
    } else {
        0.0
    };
    let fit_min = fits.iter().cloned().fold(f64::INFINITY, f64::min);
    let fit_max = fits.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // Per-card stats: presence count and mean copies WHEN PRESENT.
    let mut presence: BTreeMap<String, u32> = BTreeMap::new();
    let mut total_copies: BTreeMap<String, u32> = BTreeMap::new();
    let mut max_copies: BTreeMap<String, u32> = BTreeMap::new();
    for champ in champions {
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for id in &champ.card_ids {
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
        for (id, c) in counts {
            *presence.entry(id.clone()).or_insert(0) += 1;
            *total_copies.entry(id.clone()).or_insert(0) += c;
            let entry = max_copies.entry(id).or_insert(0);
            if c > *entry {
                *entry = c;
            }
        }
    }

    let mut rows: Vec<(String, u32, f64, u32)> = presence
        .iter()
        .map(|(id, count)| {
            let total = *total_copies.get(id).unwrap_or(&0);
            let mean = (total as f64) / (*count as f64);
            let max = *max_copies.get(id).unwrap_or(&0);
            (id.clone(), *count, mean, max)
        })
        .collect();
    rows.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.0.cmp(&b.0))
    });

    let pool_ids: BTreeSet<&str> = playable_pool.iter().map(|c| c.id.as_str()).collect();
    let unused: Vec<&str> = pool_ids
        .iter()
        .filter(|id| !presence.contains_key(**id))
        .copied()
        .collect();

    // Fitness correlation split (only meaningful at n >= 4).
    let correlation = if n >= 4 {
        let mut sorted = champions.to_vec();
        sorted.sort_by(|a, b| {
            b.fitness
                .partial_cmp(&a.fitness)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let half = n / 2;
        let top_slice = &sorted[0..half];
        let bot_slice = &sorted[n - half..];
        let presence_in = |slice: &[EvolvedDeck], id: &str| -> u32 {
            slice
                .iter()
                .filter(|c| c.card_ids.iter().any(|x| x == id))
                .count() as u32
        };
        let mut deltas: Vec<(String, i32, u32, u32)> = pool_ids
            .iter()
            .map(|id| {
                let t = presence_in(top_slice, id);
                let b = presence_in(bot_slice, id);
                let delta = (t as i32) - (b as i32);
                (id.to_string(), delta, t, b)
            })
            .filter(|(_, d, _, _)| d.abs() >= 2)
            .collect();
        deltas.sort_by_key(|x| std::cmp::Reverse(x.1));
        let top_mean = top_slice.iter().map(|c| c.fitness).sum::<f64>() / (half as f64);
        let bot_mean = bot_slice.iter().map(|c| c.fitness).sum::<f64>() / (half as f64);
        Some((half, top_mean, bot_mean, deltas))
    } else {
        None
    };

    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "tsot — champions report" }
                style { (PreEscaped(CSS)) }
            }
            body {
                h1 { "tsot — champions report" }
                div.meta {
                    div { span.k { "dir" } b { (dir) } }
                    div { span.k { "champions" } b { (n) } }
                    div { span.k { "fitness mean" } b { (format!("{fit_mean:.3}")) } }
                    div { span.k { "fitness min" } b { (format!("{fit_min:.3}")) } }
                    div { span.k { "fitness max" } b { (format!("{fit_max:.3}")) } }
                    div { span.k { "pool coverage" } b { (format!("{}/{}", presence.len(), pool_ids.len())) } }
                }

                @if n < 10 {
                    p.note {
                        "Sample size n=" (n) " is below 10. Single-digit champion counts produce noisy frequency estimates — 5/5 at mean_copies=1.0 is barely above the null hypothesis. Treat conclusions as directional, not load-bearing. 20+ independent champions at different seeds is the threshold for confident card-design calls."
                    }
                }

                h2 { "Card frequency" }
                p.note {
                    "Sorted by presence count (descending), then mean copies (descending). "
                    em { "mean_copies" } " is the mean when the card is present — a card at presence " em { "N/N" } " with mean_copies " em { "1.0" } " is universally included but never doubled. mean_copies near the cap (3.0) signals load-bearing."
                }
                table.summary {
                    thead {
                        tr {
                            th { "card id" }
                            th.num { "presence" }
                            th.num { "%" }
                            th.num { "mean_copies" }
                            th.num { "max_copies" }
                        }
                    }
                    tbody {
                        @for (id, count, mean, max) in &rows {
                            tr {
                                td { (report_style::card_cell(playable_pool, id)) }
                                td.num { (format!("{count}/{n}")) }
                                td.num { (format!("{:.0}%", 100.0 * (*count as f64) / (n as f64))) }
                                td.num { (format!("{mean:.2}")) }
                                td.num { (max) }
                            }
                        }
                    }
                }

                h2 { "Pool coverage" }
                p {
                    (presence.len()) " of " (pool_ids.len())
                    " playable cards appear in at least one champion ("
                    (unused.len()) " unused)."
                }
                @if !unused.is_empty() {
                    div.panel {
                        p { strong { "Cards never selected:" } }
                        ul {
                            @for id in &unused {
                                li { (report_style::card_cell(playable_pool, id)) }
                            }
                        }
                    }
                }

                @if let Some((half, top_mean, bot_mean, deltas)) = correlation {
                    h2 { "Fitness correlation" }
                    p.note {
                        "Champions split by fitness median. Cards with |Δpresence| ≥ 2 between the top and bottom half are shown. Positive Δ means the card skews toward winning decks."
                    }
                    div.meta {
                        div { span.k { "top half" } b { (half) } }
                        div { span.k { "top mean_fitness" } b { (format!("{top_mean:.3}")) } }
                        div { span.k { "bottom half" } b { (half) } }
                        div { span.k { "bottom mean_fitness" } b { (format!("{bot_mean:.3}")) } }
                    }
                    @if deltas.is_empty() {
                        p.muted { "No cards with |Δpresence| ≥ 2 — too little signal at this sample size." }
                    } @else {
                        table.summary {
                            thead {
                                tr {
                                    th { "card id" }
                                    th.num { "Δpresence" }
                                    th.num { "top" }
                                    th.num { "bottom" }
                                }
                            }
                            tbody {
                                @for (id, delta, t, b) in &deltas {
                                    tr {
                                        td { (report_style::card_cell(playable_pool, id)) }
                                        td.num { (if *delta >= 0 { format!("+{delta}") } else { format!("{delta}") }) }
                                        td.num { (format!("{t}/{half}")) }
                                        td.num { (format!("{b}/{half}")) }
                                    }
                                }
                            }
                        }
                    }
                }

                h2 { "Per champion" }
                table.summary {
                    thead {
                        tr {
                            th { "label" }
                            th.num { "fitness" }
                            th.num { "base_seed" }
                            th.num { "generations" }
                            th.num { "unique" }
                            th.num { "cards" }
                        }
                    }
                    tbody {
                        @for c in champions {
                            @let unique: BTreeSet<&String> = c.card_ids.iter().collect();
                            tr {
                                td { (c.label) }
                                td.num { (format!("{:.3}", c.fitness)) }
                                td.num { (format!("{:#x}", c.base_seed)) }
                                td.num { (c.generations_run) }
                                td.num { (unique.len()) }
                                td.num { (c.card_ids.len()) }
                            }
                        }
                    }
                }

                @if !game_rows.is_empty() {
                    h2 { "Game-level sample (per champion vs baselines)" }
                    p.note { "Sampled via `--sample-games N`. Lower mean turns = faster deck. Attacks/milled are per-game averages from the champion's seat." }
                    table.summary {
                        thead {
                            tr {
                                th { "champion" }
                                th.num { "games" }
                                th.num { "min turns" }
                                th.num { "median turns" }
                                th.num { "mean turns" }
                                th.num { "max turns" }
                                th.num { "attacks/g" }
                                th.num { "milled/g" }
                            }
                        }
                        tbody {
                            @for row in &game_rows {
                                tr {
                                    td { (row.label) }
                                    td.num { (row.count) }
                                    td.num { (row.min) }
                                    td.num { (row.median) }
                                    td.num { (format!("{:.1}", row.mean)) }
                                    td.num { (row.max) }
                                    td.num { (format!("{:.1}", row.attacks_avg)) }
                                    td.num { (format!("{:.1}", row.milled_avg)) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
