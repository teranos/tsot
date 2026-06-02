//! HTML report writer for `tsot evolve --html-report PATH`. Surfaces
//! the evolutionary trajectory of each card: a heatmap of card presence
//! in the population per generation. Plus the fitness line (best + mean
//! per generation) at the top so you can read climbs vs collapses.
//!
//! Reuses the shared dark-mono CSS from `report_style`.

use std::collections::BTreeSet;

use maud::{html, Markup, PreEscaped, DOCTYPE};
use tsot::card::Card;

use crate::report_style;
use tsot::sim::EvolveConfig;

pub(crate) struct EvolveReportData<'a> {
    pub cfg: &'a EvolveConfig,
    /// Card pool used by the run. Drives the hover tooltip lookup for
    /// the heatmap rows.
    pub pool: &'a [Card],
    /// Indexed by generation: 0 is the initial random pop, then one
    /// entry per generation up to and including the last.
    pub best_fitness: Vec<f64>,
    pub mean_fitness: Vec<f64>,
    /// Per-generation card presence. `freq[gen][card_id] = N` means
    /// `N` population members at generation `gen` contained at least
    /// one copy of `card_id`.
    pub freq: Vec<std::collections::BTreeMap<String, u32>>,
    /// Top-K final genomes (label, fitness) for the legend.
    pub top_final: Vec<(String, f64)>,
}

pub(crate) fn write_html_report(data: &EvolveReportData, path: &str) -> std::io::Result<()> {
    let markup = build(data);
    std::fs::write(path, markup.into_string())
}

fn build(data: &EvolveReportData) -> Markup {
    let n_gens = data.freq.len();
    let pop = data.cfg.pop_size as u32;

    // Aggregate ever-seen card ids, then sort by final-gen presence
    // descending (ties broken alphabetically). Cards never present
    // are skipped — they'd be empty rows.
    let mut all_cards: BTreeSet<String> = BTreeSet::new();
    for gen in &data.freq {
        for id in gen.keys() {
            all_cards.insert(id.clone());
        }
    }
    let final_idx = n_gens.saturating_sub(1);
    let mut sorted_cards: Vec<String> = all_cards.into_iter().collect();
    sorted_cards.sort_by(|a, b| {
        let pa = data.freq.get(final_idx).and_then(|f| f.get(a)).copied().unwrap_or(0);
        let pb = data.freq.get(final_idx).and_then(|f| f.get(b)).copied().unwrap_or(0);
        pb.cmp(&pa).then(a.cmp(b))
    });

    // Build heatmap cells: for each card, for each gen, the presence pct.
    fn cell_color(t: f64) -> String {
        // Black → accent green. Match the matrix heat-cell style.
        let r = ((1.0 - t) * 28.0 + 24.0) as u8;
        let g = (t * 160.0 + 30.0) as u8;
        let b = ((1.0 - t) * 28.0 + 24.0) as u8;
        format!("background: rgb({r},{g},{b}); color: #eee;")
    }

    // Best-fitness sparkline coordinates (SVG polyline points).
    let width = 800u32;
    let height = 120u32;
    let pad = 8u32;
    fn poly_points(values: &[f64], width: u32, height: u32, pad: u32) -> String {
        if values.is_empty() {
            return String::new();
        }
        let plot_w = width - 2 * pad;
        let plot_h = height - 2 * pad;
        let n = values.len() as f64;
        values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = pad as f64 + (i as f64 / (n - 1.0).max(1.0)) * plot_w as f64;
                let y = pad as f64 + (1.0 - v.clamp(0.0, 1.0)) * plot_h as f64;
                format!("{x:.1},{y:.1}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    let best_line = poly_points(&data.best_fitness, width, height, pad);
    let mean_line = poly_points(&data.mean_fitness, width, height, pad);

    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "tsot — evolve trajectory" }
                style { (PreEscaped(report_style::CSS)) }
                style {
                    "
                    .heatmap { border-collapse: collapse; font-size: 9px; line-height: 1.0; }
                    .heatmap th { padding: 1px 4px; font-size: 9px; }
                    .heatmap td { padding: 0; width: 18px; height: 14px; text-align: center; }
                    .heatmap td.card { padding: 1px 8px; text-align: left; min-width: 200px; width: auto; height: auto; }
                    .heatmap .gennum { font-size: 8px; color: var(--text-tertiary); }
                    .fit-svg { display: block; background: var(--bg-panel); border: 1px solid var(--border); border-radius: 3px; }
                    .fit-svg polyline.best { fill: none; stroke: var(--accent); stroke-width: 1.5; }
                    .fit-svg polyline.mean { fill: none; stroke: var(--player-a); stroke-width: 1.2; opacity: 0.8; }
                    "
                }
            }
            body {
                h1 { "tsot — evolve trajectory" }
                div.meta {
                    div { span.k { "seed" } b { (format!("{:#x}", data.cfg.base_seed)) } }
                    div { span.k { "pop" } b { (data.cfg.pop_size) } }
                    div { span.k { "gens run" } b { (n_gens.saturating_sub(1)) } }
                    div { span.k { "n_per_side" } b { (data.cfg.n_per_side) } }
                    div { span.k { "tournament k" } b { (data.cfg.tournament_k) } }
                    div { span.k { "mutation rate" } b { (format!("{:.3}", data.cfg.mutation_rate)) } }
                    div { span.k { "elite" } b { (data.cfg.elite_count) } }
                }

                h2 { "Fitness over time" }
                p.note { "Green line = best per generation (carried by elitism, monotonic with elitism on). Blue line = population mean. Y axis spans [0, 1]." }
                svg.fit-svg width=(width) height=(height) viewBox=(format!("0 0 {width} {height}")) {
                    polyline.mean points=(mean_line);
                    polyline.best points=(best_line);
                }

                h2 { "Top final genomes" }
                table.summary {
                    thead { tr { th { "rank" } th { "label" } th.num { "fitness" } } }
                    tbody {
                        @for (i, (label, fit)) in data.top_final.iter().enumerate() {
                            tr {
                                td.num { (i + 1) }
                                td { (label) }
                                td.num { (format!("{fit:.3}")) }
                            }
                        }
                    }
                }

                h2 { "Card presence over generations" }
                p.note {
                    "Y axis = card ids (sorted by final-generation presence, desc). X axis = generation. Cell intensity = fraction of " em { (pop) } "-pop containing ≥1 copy of that card at that generation. Cards rising to the top mid-run were discovered by selection; horizontal bands at the top are the universal core; cards that fade out got selected against."
                }
                div style="overflow-x: auto;" {
                    table.heatmap {
                        thead {
                            tr {
                                th { "card" }
                                @for g in 0..n_gens {
                                    th.gennum { (g) }
                                }
                            }
                        }
                        tbody {
                            @for card_id in &sorted_cards {
                                tr {
                                    td.card { (report_style::card_cell(data.pool, card_id)) }
                                    @for gen in &data.freq {
                                        @let count = gen.get(card_id).copied().unwrap_or(0);
                                        @let frac = count as f64 / pop as f64;
                                        td title=(format!("{card_id}: {count}/{pop} ({:.0}%)", frac * 100.0))
                                            style=(cell_color(frac)) {
                                            (if count > 0 { count.to_string() } else { String::new() })
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
