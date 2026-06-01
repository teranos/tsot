//! `tsot evolve` subcommand: run one generation loop of the EA against
//! the baseline gauntlet (+ optional --extra champion files), print
//! live per-generation progress with NEW BEST markers + ETA, save the
//! top-K final genomes as EvolvedDeck JSONs.

use clap::Parser;

use tsot::card::{Card, CardRegistry};

use crate::evolve_report::{self, EvolveReportData};
use crate::parse_u64_hex_or_dec;
use crate::sim::evolved_deck::EvolvedDeck;
use crate::sim::fitness::fitness_breakdown;
use crate::sim::{run_evolve, EvolveConfig};

#[derive(Parser)]
pub struct EvolveArgs {
    /// Population size per generation.
    #[arg(long, default_value_t = 50)]
    pub pop: usize,
    /// Number of generations to run.
    #[arg(long, default_value_t = 30)]
    pub gens: usize,
    /// Games per side per fitness evaluation. Total per eval =
    /// 2 × gauntlet_size × n. EA.md's measured recommendation is 10.
    #[arg(long, default_value_t = 10)]
    pub n: u32,
    /// Master seed for every random decision in the run. Accepts
    /// decimal (`60104`) or hex (`0xEAC8`).
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
    pub seed: u64,
    /// Tournament size for selection.
    #[arg(long = "tournament-k", default_value_t = 3)]
    pub tournament_k: usize,
    /// Per-slot mutation probability. With deck_len=50, rate=0.03 ≈
    /// Poisson(1.5) mutations per child.
    #[arg(long, default_value_t = 0.03)]
    pub rate: f64,
    /// Top-K individuals carry their cached fitness unchanged.
    #[arg(long, default_value_t = 1)]
    pub elite: usize,
    /// Diversity-preserving selection coefficient. Tournament reads
    /// `fitness - alpha · mean_jaccard_to_others`. `0.0` (default) is
    /// vanilla selection — byte-identical to pre-diversity-aware runs.
    /// Useful range roughly `[0.05, 0.3]`. Elitism still carries by
    /// raw fitness so the best-of-generation trace stays monotonic.
    #[arg(long = "diversity-alpha", default_value_t = 0.0)]
    pub diversity_alpha: f64,
    /// Stop after K consecutive generations at fitness 1.0.
    #[arg(long = "stop-at-ceiling")]
    pub stop_at_ceiling: Option<usize>,
    /// Skip building the variant gauntlet. Requires at least one --extra.
    #[arg(long = "no-variants")]
    pub no_variants: bool,
    /// Path to a saved evolved deck to load as a gauntlet opponent.
    /// Can be passed multiple times.
    #[arg(long = "extra", value_name = "PATH")]
    pub extras: Vec<String>,
    /// Path to save the rank-1 deck. Defaults to `evolved-{seed:x}.json`.
    /// When --save-top K > 1, the rank suffix is inserted before the
    /// extension: `evolved-{seed:x}-rank1.json`, `-rank2.json`, etc.
    #[arg(long, value_name = "PATH")]
    pub save: Option<String>,
    /// Label embedded in the saved deck. Defaults to `evo_{seed:x}`.
    #[arg(long = "save-label")]
    pub save_label: Option<String>,
    /// Save the top-K final genomes (default 1).
    #[arg(long = "save-top", default_value_t = 1)]
    pub save_top: usize,
    /// Path to write an HTML report of the evolutionary trajectory
    /// (per-generation card-presence heatmap + fitness line chart).
    /// Default `evolve-report.html`. Use `-` to skip.
    #[arg(long = "html-report", default_value = "evolve-report.html")]
    pub html_report: String,
}

/// Pretty-print a deck's card-id list grouped by count.
fn print_deck_listing(header: &str, deck: &[String]) {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for id in deck {
        *counts.entry(id.as_str()).or_insert(0) += 1;
    }
    println!(
        "=== {header} ({} cards, {} unique) ===",
        deck.len(),
        counts.len()
    );
    let mut sorted: Vec<(&&str, &u32)> = counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (id, n) in sorted {
        println!("  {n}x {id}");
    }
}

/// Inject a `-rank{N}` suffix before the extension when saving top-K
/// genomes so multiple ranks don't collide. K=1 returns `base`
/// unchanged for back-compat.
fn rank_suffixed_path(base: &str, rank: usize, total: usize) -> String {
    if total <= 1 {
        return base.to_string();
    }
    let p = std::path::Path::new(base);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("evolved");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("json");
    match p.parent().and_then(|x| x.to_str()) {
        Some(parent) if !parent.is_empty() => format!("{parent}/{stem}-rank{rank}.{ext}"),
        _ => format!("{stem}-rank{rank}.{ext}"),
    }
}

pub fn run_ea(
    registry: &CardRegistry,
    playable_pool: &[Card],
    args: &EvolveArgs,
) -> mlua::Result<()> {
    let cfg = EvolveConfig {
        pop_size: args.pop,
        generations: args.gens,
        n_per_side: args.n,
        base_seed: args.seed,
        deck_len: 50,
        per_card_cap: 3,
        tournament_k: args.tournament_k,
        mutation_rate: args.rate,
        elite_count: args.elite,
        stop_at_ceiling: args.stop_at_ceiling,
        pinned_card_id: None,
        pinned_count: 0,
        diversity_alpha: args.diversity_alpha,
    };

    println!();
    println!("=== EA mode ===");
    println!(
        "  pop={} gens={} n={} seed={:#x} tournament_k={} rate={} elite={} stop_at_ceiling={:?} diversity_alpha={}",
        cfg.pop_size,
        cfg.generations,
        cfg.n_per_side,
        cfg.base_seed,
        cfg.tournament_k,
        cfg.mutation_rate,
        cfg.elite_count,
        cfg.stop_at_ceiling,
        cfg.diversity_alpha,
    );
    println!();

    let no_variants = args.no_variants;
    let (mut gauntlet, mut gauntlet_labels): (Vec<Vec<Card>>, Vec<String>) = if no_variants {
        println!("Gauntlet: baselines skipped (--no-variants)");
        (Vec::new(), Vec::new())
    } else {
        let mut g: Vec<Vec<Card>> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        let baselines_dir = std::path::Path::new("baselines");
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(baselines_dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("json") {
                    paths.push(p);
                }
            }
        }
        paths.sort();
        for path in &paths {
            match EvolvedDeck::load(path) {
                Ok(saved) => match saved.to_cards(registry) {
                    Ok(cards) => {
                        labels.push(saved.label.clone());
                        g.push(cards);
                    }
                    Err(e) => eprintln!("  ! baseline {} unloadable: {e}", path.display()),
                },
                Err(e) => eprintln!("  ! baseline {} unparseable: {e}", path.display()),
            }
        }
        println!(
            "Gauntlet: {} baseline decks loaded from {}",
            g.len(),
            baselines_dir.display(),
        );
        for (label, path) in labels.iter().zip(&paths) {
            println!("  + {label} ({})", path.display());
        }
        (g, labels)
    };
    // Snapshot baseline count BEFORE adding extras, so the summary
    // print can show baseline / extras counts independently of how
    // many extras successfully materialized (some can fail when their
    // card-id set references cards removed from the pool — see
    // LIMITATIONS, "Champion artifacts age with the card pool").
    let baseline_count = if no_variants { 0 } else { gauntlet.len() };
    let mut extras_loaded = 0usize;
    for path in &args.extras {
        match EvolvedDeck::load(std::path::Path::new(path)) {
            Ok(saved) => match saved.to_cards(registry) {
                Ok(cards) => {
                    println!(
                        "  + extra: {} (label={}, prior fitness={:.3}, base_seed={:#x})",
                        path, saved.label, saved.fitness, saved.base_seed
                    );
                    gauntlet_labels.push(saved.label);
                    gauntlet.push(cards);
                    extras_loaded += 1;
                }
                Err(e) => eprintln!("  ! failed to materialize {path}: {e}"),
            },
            Err(e) => eprintln!("  ! failed to load {path}: {e}"),
        }
    }
    if !args.extras.is_empty() {
        let extras_requested = args.extras.len();
        let extras_failed = extras_requested - extras_loaded;
        if extras_failed > 0 {
            println!(
                "Gauntlet now {} decks total ({} baselines + {} extras; {} extra(s) skipped — see warnings above)",
                gauntlet.len(),
                baseline_count,
                extras_loaded,
                extras_failed,
            );
        } else {
            println!(
                "Gauntlet now {} decks total ({} baselines + {} extras)",
                gauntlet.len(),
                baseline_count,
                extras_loaded,
            );
        }
    }
    if gauntlet.is_empty() {
        eprintln!(
            "error: gauntlet is empty — either populate baselines/ or pass --extra PATH when --no-variants is set."
        );
        std::process::exit(2);
    }
    let evals_per_gen = cfg.pop_size - cfg.elite_count.min(cfg.pop_size);
    let games_per_eval = 2 * gauntlet.len() as u32 * cfg.n_per_side;
    let total_games = (cfg.pop_size + evals_per_gen * cfg.generations) as u64
        * games_per_eval as u64;
    println!(
        "  budget: ~{} games total ({} per fitness eval × {} evals, {}-deck gauntlet)",
        total_games,
        games_per_eval,
        cfg.pop_size + evals_per_gen * cfg.generations,
        gauntlet.len(),
    );
    println!();

    let t_start = std::time::Instant::now();
    let mut t_prev = t_start;
    let mut prev_best: Option<f64> = None;
    let total_gens = cfg.generations;
    let result = {
        let cb = &mut |gen: usize, pop: &[(Vec<String>, f64)]| {
            let now = std::time::Instant::now();
            let took = now - t_prev;
            let total = now - t_start;
            let best = pop[0].1;
            let mean: f64 = pop.iter().map(|(_, f)| *f).sum::<f64>() / pop.len() as f64;
            let min = pop.last().map(|(_, f)| *f).unwrap_or(0.0);
            let new_best = match prev_best {
                Some(p) if best > p + f64::EPSILON => " | NEW BEST",
                _ => "",
            };
            prev_best = Some(best);
            let eta = if gen > 0 && gen < total_gens {
                let per_gen = total.as_secs_f64() / (gen as f64);
                let remaining_secs = per_gen * (total_gens - gen) as f64;
                format!(" | ETA {remaining_secs:>5.0}s")
            } else {
                String::new()
            };
            println!(
                "  gen {gen:>2}/{total_gens} | best={best:.3} mean={mean:.3} min={min:.3} | took {took:>5.1?} | total {total:>5.1?}{eta}{new_best}"
            );
            t_prev = now;
        };
        run_evolve(registry, playable_pool, &gauntlet, &cfg, cb)
    };
    let elapsed = t_start.elapsed();

    println!();
    let gens_run = result.best_per_generation.len().saturating_sub(1);
    let stopped_early = gens_run < cfg.generations;
    println!(
        "Done in {:.2?} ({} generations{}, {} pop_size)",
        elapsed,
        gens_run,
        if stopped_early {
            format!(" — early-stopped at ceiling after {gens_run} of {} planned", cfg.generations)
        } else {
            String::new()
        },
        cfg.pop_size
    );
    println!();
    println!("=== Top 5 final-population genomes ===");
    let top_n = result.final_population.len().min(5);
    let top_sets: Vec<std::collections::BTreeSet<String>> = result
        .final_population
        .iter()
        .take(top_n)
        .map(|(g, _)| g.iter().cloned().collect())
        .collect();
    for (rank, (genome, fit)) in result.final_population.iter().take(top_n).enumerate() {
        println!();
        print_deck_listing(&format!("rank {} (fitness {:.3})", rank + 1, fit), genome);
        match fitness_breakdown(
            registry,
            genome,
            &gauntlet,
            cfg.n_per_side,
            cfg.base_seed.wrapping_add(0xB1EAD_u64.wrapping_mul(rank as u64 + 1)),
        ) {
            Ok(b) => {
                print!("  per-opponent:");
                for (label, v) in gauntlet_labels.iter().zip(b.per_opponent.iter()) {
                    print!("  {label}={v:.2}");
                }
                println!("    (re-eval total {:.3})", b.total);
            }
            Err(e) => println!("  per-opponent: <error: {e}>"),
        }
        // Pairwise Jaccard against the other top-N — the diversity-
        // penalty audit. A row of `0.9+` against every other rank means
        // the top-N are slot-variations of one attractor; a row of
        // `~0.4` means they're distinct archetypes.
        if top_n > 1 {
            print!("  jaccard vs top-{top_n}:");
            let mut sum = 0.0_f64;
            let mut paired = 0u32;
            for j in 0..top_n {
                if j == rank {
                    continue;
                }
                let jacc = crate::sim::diversity::jaccard(&top_sets[rank], &top_sets[j]);
                print!("  r{}={:.2}", j + 1, jacc);
                sum += jacc;
                paired += 1;
            }
            println!("    (mean {:.2})", sum / paired as f64);
        }
    }

    let top_k = args.save_top.max(1).min(result.final_population.len());
    if top_k > 0 {
        let base_path = args
            .save
            .clone()
            .unwrap_or_else(|| format!("evolved-{:x}.json", cfg.base_seed));
        let base_label = args
            .save_label
            .clone()
            .unwrap_or_else(|| format!("evo_{:x}", cfg.base_seed));
        println!();
        for (rank_idx, (genome, fit)) in
            result.final_population.iter().take(top_k).enumerate()
        {
            let rank = rank_idx + 1;
            let path = rank_suffixed_path(&base_path, rank, top_k);
            let label = if top_k == 1 {
                base_label.clone()
            } else {
                format!("{base_label}_r{rank}")
            };
            let saved = EvolvedDeck {
                label,
                fitness: *fit,
                base_seed: cfg.base_seed,
                generations_run: gens_run,
                card_ids: genome.clone(),
            };
            match saved.save(std::path::Path::new(&path)) {
                Ok(()) => println!("Saved rank-{rank} deck to {path}  (fitness {fit:.3})"),
                Err(e) => eprintln!("failed to save rank-{rank} deck to {path}: {e}"),
            }
        }
    }

    // HTML trajectory report: card-presence heatmap + fitness lines.
    if args.html_report != "-" {
        let best_fitness: Vec<f64> = result
            .best_per_generation
            .iter()
            .map(|(_, f)| *f)
            .collect();
        let top_final: Vec<(String, f64)> = result
            .final_population
            .iter()
            .take(5)
            .enumerate()
            .map(|(rank, (_, fit))| (format!("rank {}", rank + 1), *fit))
            .collect();
        let data = EvolveReportData {
            cfg: &cfg,
            pool: playable_pool,
            best_fitness,
            mean_fitness: result.per_gen_mean_fitness.clone(),
            freq: result.per_gen_card_freq.clone(),
            top_final,
        };
        match evolve_report::write_html_report(&data, &args.html_report) {
            Ok(()) => println!("Evolve trajectory written to {}", args.html_report),
            Err(e) => eprintln!("failed to write evolve trajectory: {e}"),
        }
    }

    Ok(())
}
