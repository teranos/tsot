//! `tsot champions-report` subcommand: aggregates card-level signal
//! across a directory of saved EvolvedDeck files. Prints frequency,
//! mean copies, pool coverage, fitness correlation, and Jaccard
//! clusters. Optionally samples real games (`--sample-games N`) to
//! surface per-champion turn count + action data.

use std::collections::BTreeMap;

use clap::Parser;
use rand::rngs::StdRng;
use rand::SeedableRng;

use tsot::card::{Card, CardRegistry};
use tsot::game::GameState;

use crate::champions_report;
use crate::parse_u64_hex_or_dec;
use crate::sim;
use crate::sim::evolved_deck::EvolvedDeck;

#[derive(Parser)]
pub struct ChampionsReportArgs {
    /// Directory containing saved EvolvedDeck JSON files.
    #[arg(long, value_name = "DIR")]
    pub dir: String,
    /// Show only the top N cards by frequency in stdout (default: all).
    #[arg(long)]
    pub top: Option<usize>,
    /// Also write a full HTML report to this path.
    #[arg(long, value_name = "PATH")]
    pub html: Option<String>,
    /// Jaccard-similarity threshold for clustering. Two decks are
    /// linked if `|A ∩ B| / |A ∪ B|` on card-id sets exceeds this.
    /// Single-linkage clustering then groups linked decks transitively.
    #[arg(long = "cluster-threshold", default_value_t = 0.7)]
    pub cluster_threshold: f64,
    /// Sample games per champion-vs-baseline pairing. Default 0 = skip
    /// game-level stats (fast, card-level only). With N > 0, each
    /// champion plays N games vs each baseline (both seats) and the
    /// report includes per-champion turn counts + action totals.
    #[arg(long = "sample-games", default_value_t = 0)]
    pub sample_games: u32,
    /// Directory of baseline opponents for the sample games. Default
    /// `baselines/`. Only used when `--sample-games > 0`.
    #[arg(long = "baselines", default_value = "baselines")]
    pub baselines: String,
    /// Seed for the sample-game RNG (for reproducibility).
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
    pub seed: u64,
}

/// Per-champion game-level aggregate from `--sample-games`. Empty
/// `turns` means no sample run was done.
#[derive(Default, Clone)]
pub struct ChampGameStats {
    pub turns: Vec<u32>,
    pub attacks: u64,
    pub deaths: u64,
    pub milled: u64,
    pub played: u64,
}

pub fn run_champions_report(
    registry: &CardRegistry,
    playable_pool: &[Card],
    args: &ChampionsReportArgs,
) -> mlua::Result<()> {
    let entries = match std::fs::read_dir(&args.dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: cannot read directory {}: {e}", args.dir);
            std::process::exit(2);
        }
    };
    let mut champions: Vec<EvolvedDeck> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        match EvolvedDeck::load(&path) {
            Ok(d) => champions.push(d),
            Err(e) => eprintln!("skip {}: {e}", path.display()),
        }
    }
    if champions.is_empty() {
        eprintln!("error: no champions found in {} (looking for *.json)", args.dir);
        std::process::exit(2);
    }
    let n = champions.len();
    println!();
    println!("=== Champions report: {n} decks from {} ===", args.dir);
    let fits: Vec<f64> = champions.iter().map(|c| c.fitness).collect();
    let fit_mean: f64 = fits.iter().sum::<f64>() / (n as f64);
    let fit_min = fits.iter().cloned().fold(f64::INFINITY, f64::min);
    let fit_max = fits.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    println!("Fitness: mean={fit_mean:.3}  min={fit_min:.3}  max={fit_max:.3}");
    println!();

    let mut presence: BTreeMap<String, u32> = BTreeMap::new();
    let mut total_copies: BTreeMap<String, u32> = BTreeMap::new();
    let mut max_copies: BTreeMap<String, u32> = BTreeMap::new();
    for champ in &champions {
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
    let display_rows = match args.top {
        Some(k) => rows.iter().take(k).cloned().collect::<Vec<_>>(),
        None => rows.clone(),
    };

    println!(
        "Card frequency across {n} champions  (mean_copies = mean WHEN PRESENT):"
    );
    println!(
        "  {:<35} {:>10} {:>14} {:>11}",
        "card_id", "in N/N", "mean_copies", "max_copies"
    );
    for (id, count, mean, max) in &display_rows {
        let pct = 100.0 * (*count as f64) / (n as f64);
        println!(
            "  {:<35} {:>5}/{:<3}  ({:>3.0}%)  {:>10.2}  {:>10}",
            id, count, n, pct, mean, max
        );
    }

    let pool_ids: std::collections::BTreeSet<&str> =
        playable_pool.iter().map(|c| c.id.as_str()).collect();
    let unused: Vec<&str> = pool_ids
        .iter()
        .filter(|id| !presence.contains_key(**id))
        .copied()
        .collect();
    println!();
    println!(
        "Pool coverage: {}/{} playable cards appear in at least one champion ({} unused)",
        presence.len(),
        pool_ids.len(),
        unused.len(),
    );
    if !unused.is_empty() {
        println!("Unused cards (never selected across {n} champions):");
        for id in &unused {
            println!("  {id}");
        }
    }

    if n >= 4 {
        let mut sorted = champions.clone();
        sorted.sort_by(|a, b| b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal));
        let half = n / 2;
        let top = &sorted[0..half];
        let bot = &sorted[n - half..];
        let presence_in = |slice: &[EvolvedDeck], id: &str| -> u32 {
            slice
                .iter()
                .filter(|c| c.card_ids.iter().any(|x| x == id))
                .count() as u32
        };
        let mut deltas: Vec<(String, i32, u32, u32)> = pool_ids
            .iter()
            .map(|id| {
                let t = presence_in(top, id);
                let b = presence_in(bot, id);
                let delta = (t as i32) - (b as i32);
                (id.to_string(), delta, t, b)
            })
            .filter(|(_, d, _, _)| d.abs() >= 2)
            .collect();
        deltas.sort_by_key(|b| std::cmp::Reverse(b.1));
        println!();
        println!(
            "Fitness correlation (top {half} vs bottom {half} champions by fitness):"
        );
        println!(
            "  top mean_fitness={:.3}   bottom mean_fitness={:.3}",
            top.iter().map(|c| c.fitness).sum::<f64>() / (half as f64),
            bot.iter().map(|c| c.fitness).sum::<f64>() / (half as f64),
        );
        if deltas.is_empty() {
            println!("  (no cards with |Δpresence| >= 2 — too little signal at this sample size)");
        } else {
            for (id, delta, t, b) in &deltas {
                let sign = if *delta >= 0 { '+' } else { '-' };
                println!(
                    "  {sign}{:>2}   {:<35}  (top {}/{half}, bottom {}/{half})",
                    delta.abs(),
                    id,
                    t,
                    b,
                );
            }
        }
    }

    let champ_sets: Vec<std::collections::BTreeSet<&str>> = champions
        .iter()
        .map(|c| c.card_ids.iter().map(|s| s.as_str()).collect())
        .collect();
    let mut parent: Vec<usize> = (0..champions.len()).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut total_pairs = 0u32;
    let mut linked_pairs = 0u32;
    for i in 0..champions.len() {
        for j in (i + 1)..champions.len() {
            total_pairs += 1;
            let jacc = crate::sim::diversity::jaccard(&champ_sets[i], &champ_sets[j]);
            if jacc > args.cluster_threshold {
                linked_pairs += 1;
                let ri = find(&mut parent, i);
                let rj = find(&mut parent, j);
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..champions.len() {
        let r = find(&mut parent, i);
        clusters.entry(r).or_default().push(i);
    }
    let mut cluster_list: Vec<Vec<usize>> = clusters.into_values().collect();
    cluster_list.sort_by_key(|c| std::cmp::Reverse(c.len()));

    println!();
    println!(
        "Clusters (Jaccard threshold = {:.2}, {linked_pairs}/{total_pairs} pairs linked):",
        args.cluster_threshold
    );
    println!(
        "  {} distinct attractors among {} champions",
        cluster_list.len(),
        champions.len()
    );
    for (idx, members) in cluster_list.iter().enumerate() {
        let rep_idx = *members
            .iter()
            .max_by(|a, b| {
                champions[**a]
                    .fitness
                    .partial_cmp(&champions[**b].fitness)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        let rep = &champions[rep_idx];
        let unique_count = champ_sets[rep_idx].len();
        println!();
        println!(
            "  Cluster {} ({} members, representative fitness={:.3}, {} unique cards):",
            idx + 1,
            members.len(),
            rep.fitness,
            unique_count
        );
        for &m_idx in members {
            let c = &champions[m_idx];
            let marker = if m_idx == rep_idx { "*" } else { " " };
            println!(
                "    {marker} {:<35}  fit={:.3}  seed={:#x}",
                c.label, c.fitness, c.base_seed,
            );
        }
    }

    let per_champ_game_stats: Vec<ChampGameStats> = if args.sample_games > 0 {
        use rand::Rng;
        let baselines_dir = std::path::Path::new(&args.baselines);
        let mut baseline_decks: Vec<Vec<Card>> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(baselines_dir) {
            let mut paths: Vec<std::path::PathBuf> = rd
                .flatten()
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
                .map(|e| e.path())
                .collect();
            paths.sort();
            for p in &paths {
                if let Ok(d) = EvolvedDeck::load(p) {
                    if let Ok(cards) = d.to_cards(registry) {
                        baseline_decks.push(cards);
                    }
                }
            }
        }
        if baseline_decks.is_empty() {
            eprintln!(
                "warning: --sample-games {} requested but no baselines in {} — skipping",
                args.sample_games,
                baselines_dir.display()
            );
            Vec::new()
        } else {
            println!();
            println!(
                "Sampling {} games × {} baselines × 2 seats = {} games per champion …",
                args.sample_games,
                baseline_decks.len(),
                args.sample_games * baseline_decks.len() as u32 * 2
            );
            let t_sample = std::time::Instant::now();
            let mut rng = StdRng::seed_from_u64(args.seed);
            let mut out: Vec<ChampGameStats> = Vec::with_capacity(champions.len());
            for champ in &champions {
                let cards = match EvolvedDeck::to_cards(champ, registry) {
                    Ok(c) => c,
                    Err(_) => {
                        out.push(ChampGameStats::default());
                        continue;
                    }
                };
                let mut s = ChampGameStats::default();
                for opp in &baseline_decks {
                    for _ in 0..args.sample_games {
                        for swap in [false, true] {
                            let (a, b) = if swap {
                                (opp.clone(), cards.clone())
                            } else {
                                (cards.clone(), opp.clone())
                            };
                            let state = GameState::new(a, b);
                            let mut game_rng = StdRng::seed_from_u64(rng.gen());
                            let mut log: Vec<String> = Vec::new();
                            let (stats, _) =
                                sim::run_game(state, &mut game_rng, &mut log, registry.lua());
                            s.turns.push(stats.turns);
                            if swap {
                                s.attacks += stats.b_attacks as u64;
                                s.deaths += stats.b_deaths as u64;
                                s.milled += stats.b_milled_to_exile as u64;
                                s.played += stats.b_played as u64;
                            } else {
                                s.attacks += stats.a_attacks as u64;
                                s.deaths += stats.a_deaths as u64;
                                s.milled += stats.a_milled_to_exile as u64;
                                s.played += stats.a_played as u64;
                            }
                        }
                    }
                }
                out.push(s);
            }
            println!("Done sampling in {:.2?}", t_sample.elapsed());
            out
        }
    } else {
        Vec::new()
    };

    if !per_champ_game_stats.is_empty() {
        println!();
        println!("Per-champion game-level stats (vs baselines):");
        println!(
            "  {:<25}  {:>6}  {:>6}  {:>8}  {:>8}  {:>8}",
            "champion", "min_t", "max_t", "mean_t", "attacks", "milled"
        );
        for (champ, gs) in champions.iter().zip(per_champ_game_stats.iter()) {
            let mut ts = gs.turns.clone();
            ts.sort_unstable();
            if ts.is_empty() {
                continue;
            }
            let count = ts.len() as f64;
            let min_t = *ts.first().unwrap();
            let max_t = *ts.last().unwrap();
            let mean_t = ts.iter().sum::<u32>() as f64 / count;
            println!(
                "  {:<25}  {:>6}  {:>6}  {:>8.1}  {:>8.1}  {:>8.1}",
                if champ.label.len() > 25 { &champ.label[..25] } else { &champ.label },
                min_t,
                max_t,
                mean_t,
                gs.attacks as f64 / count,
                gs.milled as f64 / count
            );
        }
    }

    if let Some(html_path) = &args.html {
        match champions_report::write_html_report(
            &champions,
            playable_pool,
            &args.dir,
            html_path,
            &per_champ_game_stats,
        ) {
            Ok(()) => {
                println!();
                println!("HTML report written to {html_path}");
            }
            Err(e) => eprintln!("\nfailed to write HTML report to {html_path}: {e}"),
        }
    }

    Ok(())
}
