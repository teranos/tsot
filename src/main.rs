mod champions_report;
mod report;
mod sim;

use clap::{Parser, Subcommand};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::collections::BTreeSet;
use std::path::Path;
use tsot::card::{Card, CardRegistry, CardType, CostSource};
use tsot::game::GameState;

use sim::{
    build_gauntlet, build_random_deck, mandatory_for_variant, print_aggregate, run_evolve,
    run_game, variant_label, variant_pool, DeckToken, DeckVariant, EvolveConfig, GameStats, Side,
    GAUNTLET_MASTER_SEED, VARIANTS,
};
use sim::evolved_deck::EvolvedDeck;
use sim::fitness::fitness_breakdown;

#[derive(Parser)]
#[command(
    name = "tsot",
    about = "The Symbols of Teranos — 1v1 card game simulator",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the variant matchup sweep (the default behavior).
    Matchup,
    /// Evolve a deck via genetic algorithm against a gauntlet.
    Evolve(EvolveArgs),
    /// Aggregate stats across a directory of saved champions.
    ChampionsReport(ChampionsReportArgs),
}

#[derive(Parser)]
struct ChampionsReportArgs {
    /// Directory containing saved EvolvedDeck JSON files.
    #[arg(long, value_name = "DIR")]
    dir: String,
    /// Show only the top N cards by frequency in stdout (default: all).
    #[arg(long)]
    top: Option<usize>,
    /// Also write a full HTML report to this path.
    #[arg(long, value_name = "PATH")]
    html: Option<String>,
}

#[derive(Parser)]
struct EvolveArgs {
    /// Population size per generation.
    #[arg(long, default_value_t = 50)]
    pop: usize,
    /// Number of generations to run.
    #[arg(long, default_value_t = 30)]
    gens: usize,
    /// Games per side per fitness evaluation. Total per eval =
    /// 2 × gauntlet_size × n. EA.md's measured recommendation is 10.
    #[arg(long, default_value_t = 10)]
    n: u32,
    /// Master seed for every random decision in the run.
    #[arg(long, default_value_t = 0xEA_C8)]
    seed: u64,
    /// Tournament size for selection.
    #[arg(long = "tournament-k", default_value_t = 3)]
    tournament_k: usize,
    /// Per-slot mutation probability. With deck_len=50, rate=0.03 ≈
    /// Poisson(1.5) mutations per child.
    #[arg(long, default_value_t = 0.03)]
    rate: f64,
    /// Top-K individuals carry their cached fitness unchanged.
    #[arg(long, default_value_t = 1)]
    elite: usize,
    /// Stop after K consecutive generations at fitness 1.0.
    #[arg(long = "stop-at-ceiling")]
    stop_at_ceiling: Option<usize>,
    /// Skip building the variant gauntlet. Requires at least one --extra.
    #[arg(long = "no-variants")]
    no_variants: bool,
    /// Path to a saved evolved deck to load as a gauntlet opponent.
    /// Can be passed multiple times.
    #[arg(long = "extra", value_name = "PATH")]
    extras: Vec<String>,
    /// Path to save the rank-1 deck. Defaults to `evolved-{seed:x}.json`.
    /// When --save-top K > 1, the rank suffix is inserted before the
    /// extension: `evolved-{seed:x}-rank1.json`, `-rank2.json`, etc.
    #[arg(long, value_name = "PATH")]
    save: Option<String>,
    /// Label embedded in the saved deck. Defaults to `evo_{seed:x}`.
    #[arg(long = "save-label")]
    save_label: Option<String>,
    /// Save the top-K final genomes (default 1, matches the previous
    /// single-rank-1 behavior). K=5 is a reasonable starting point for
    /// feeding more samples into champions-report at no extra compute.
    #[arg(long = "save-top", default_value_t = 1)]
    save_top: usize,
}

/// When saving top-K genomes, inject a `-rank{N}` suffix before the
/// extension so multiple ranks don't collide. K=1 returns `base`
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

/// Master seed for the sim's RNG. Default: fresh per run from system
/// entropy. Override via env var `TSOT_SEED=<integer>`.
fn pick_seed() -> u64 {
    std::env::var("TSOT_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| {
            use rand::RngCore;
            StdRng::from_entropy().next_u64()
        })
}

/// Games per matchup cell. Override with `TSOT_GAMES_PER_MATCHUP=<n>`.
const DEFAULT_GAMES_PER_MATCHUP: usize = 100;

fn games_per_matchup() -> usize {
    std::env::var("TSOT_GAMES_PER_MATCHUP")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_GAMES_PER_MATCHUP)
}

/// Pretty-print a deck's card-id list grouped by count. 50-card deck →
/// 13-25 unique ids → quick visual inspection of the deck's shape.
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
    // Sort by count descending, then name ascending — stable, readable.
    let mut sorted: Vec<(&&str, &u32)> = counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (id, n) in sorted {
        println!("  {n}x {id}");
    }
}

/// EA mode entry point. Builds a [`EvolveConfig`] from CLI args, builds
/// the gauntlet (variants and/or loaded evolved decks), runs evolution,
/// prints live per-generation progress + final top-5 genomes with per-
/// opponent breakdowns, saves the rank-1 deck to disk.
/// Aggregate card-level signal across all saved EvolvedDeck files in a
/// directory. Prints frequency, mean copies, and fitness correlation —
/// which cards consistently survive selection across many runs.
fn run_champions_report(
    playable_pool: &[Card],
    args: &ChampionsReportArgs,
) -> mlua::Result<()> {
    use std::collections::BTreeMap;
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

    // Per-card stats: in how many champions does each id appear, and
    // mean copies WHEN PRESENT (not when averaged across all champions,
    // which would smear it).
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
    // Sort by presence count desc, then mean_copies desc, then name asc.
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

    // Pool coverage: cards in the playable pool with zero appearances.
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

    // Fitness correlation: split champions into top half and bottom
    // half by fitness, show which cards skew toward the top.
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
            .filter(|(_, d, _, _)| d.abs() >= 2) // skip noise
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

    if let Some(html_path) = &args.html {
        match champions_report::write_html_report(&champions, playable_pool, &args.dir, html_path) {
            Ok(()) => {
                println!();
                println!("HTML report written to {html_path}");
            }
            Err(e) => eprintln!("\nfailed to write HTML report to {html_path}: {e}"),
        }
    }

    Ok(())
}

fn run_ea(
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
    };

    println!();
    println!("=== EA mode ===");
    println!(
        "  pop={} gens={} n={} seed={:#x} tournament_k={} rate={} elite={} stop_at_ceiling={:?}",
        cfg.pop_size,
        cfg.generations,
        cfg.n_per_side,
        cfg.base_seed,
        cfg.tournament_k,
        cfg.mutation_rate,
        cfg.elite_count,
        cfg.stop_at_ceiling,
    );
    let evals_per_gen = cfg.pop_size - cfg.elite_count.min(cfg.pop_size);
    let games_per_eval = 2 * VARIANTS.len() as u32 * cfg.n_per_side;
    let total_games = (cfg.pop_size + evals_per_gen * cfg.generations) as u64
        * games_per_eval as u64;
    println!(
        "  budget: ~{} games total ({} per fitness eval × {} evals)",
        total_games,
        games_per_eval,
        cfg.pop_size + evals_per_gen * cfg.generations,
    );
    println!();

    let no_variants = args.no_variants;
    let (mut gauntlet, mut gauntlet_labels): (Vec<Vec<Card>>, Vec<String>) = if no_variants {
        println!("Gauntlet: variants skipped (--no-variants)");
        (Vec::new(), Vec::new())
    } else {
        let g = build_gauntlet(playable_pool, GAUNTLET_MASTER_SEED);
        let labels: Vec<String> = VARIANTS
            .iter()
            .map(|v| variant_label(*v).to_string())
            .collect();
        println!(
            "Gauntlet: {} variant decks built from master_seed={:#x}",
            g.len(),
            GAUNTLET_MASTER_SEED,
        );
        (g, labels)
    };
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
                }
                Err(e) => eprintln!("  ! failed to materialize {path}: {e}"),
            },
            Err(e) => eprintln!("  ! failed to load {path}: {e}"),
        }
    }
    if !args.extras.is_empty() {
        let variant_count = if no_variants { 0 } else { VARIANTS.len() };
        println!(
            "Gauntlet now {} decks total ({} variants + {} extras)",
            gauntlet.len(),
            variant_count,
            gauntlet.len() - variant_count,
        );
    }
    if gauntlet.is_empty() {
        eprintln!(
            "error: gauntlet is empty — pass --extra PATH at least once when --no-variants is set, otherwise every fitness eval is 0.0 and the EA has no signal."
        );
        std::process::exit(2);
    }
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
    // Diagnostic re-eval with a fresh seed per rank so the per-opponent
    // breakdown numbers are independent of the seed the EA happened to
    // draw during selection. The breakdown's .total will be close to but
    // not exactly the rank's listed fitness (within-genome stddev ~0.043
    // at n=10 from EA.md's variance measurement).
    for (rank, (genome, fit)) in result.final_population.iter().take(5).enumerate() {
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
    }

    // Save the top-K decks to disk so they can be fed back as gauntlet
    // opponents or aggregated by champions-report.
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

    Ok(())
}

fn main() -> mlua::Result<()> {
    // Parse args FIRST so `--help` / `--version` short-circuit before the
    // 70+ Lua cards load. Otherwise `tsot evolve --help` takes a second
    // just to print help text.
    let cli = Cli::parse();
    let registry = CardRegistry::load(Path::new("cards"))?;
    // Deck-construction pool: playable card types with supported cost sources.
    let playable_pool: Vec<Card> = registry
        .cards()
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                CardType::Creature
                    | CardType::Spell
                    | CardType::Artifact
                    | CardType::Mutation
            )
        })
        .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
        .filter(|c| {
            c.cost.iter().all(|cc| {
                matches!(
                    cc.source,
                    CostSource::Hand
                        | CostSource::Mill
                        | CostSource::Graveyard
                        | CostSource::Sacrifice
                )
            })
        })
        .cloned()
        .collect();
    let creature_count = playable_pool
        .iter()
        .filter(|c| matches!(c.kind, CardType::Creature))
        .count();
    let instant_count = playable_pool
        .iter()
        .filter(|c| c.kind == CardType::Spell && c.timing == Some(tsot::Timing::Instant))
        .count();
    let sorcery_count = playable_pool
        .iter()
        .filter(|c| c.kind == CardType::Spell && c.timing == Some(tsot::Timing::Sorcery))
        .count();

    println!(
        "loaded {} cards ({} creatures + {} instants + {} sorceries in deck pool)",
        registry.cards().len(),
        creature_count,
        instant_count,
        sorcery_count,
    );

    if let Some(Command::Evolve(args)) = &cli.command {
        return run_ea(&registry, &playable_pool, args);
    }
    if let Some(Command::ChampionsReport(args)) = &cli.command {
        return run_champions_report(&playable_pool, args);
    }

    let seed = pick_seed();
    println!("seed: {seed}");
    let mut rng = StdRng::seed_from_u64(seed);
    let mut all: Vec<GameStats> = Vec::new();
    let mut last_log: Vec<String> = Vec::new();

    let replay_out_path = std::env::var("TSOT_REPLAY_OUT").ok();

    let t0 = std::time::Instant::now();
    let mut last_deck_a_ids: Vec<String> = Vec::new();
    let mut last_deck_b_ids: Vec<String> = Vec::new();
    let mut last_journal: tsot::game::Journal = tsot::game::Journal::new();
    let pools: Vec<(DeckVariant, Vec<Card>)> = VARIANTS
        .iter()
        .map(|v| (*v, variant_pool(&playable_pool, *v)))
        .collect();
    let games_per_cell = games_per_matchup();
    let total_games = games_per_cell * VARIANTS.len() * VARIANTS.len();
    println!();
    println!("Variant pools:");
    for (v, pool) in &pools {
        println!("  {} — {} cards", variant_label(*v), pool.len());
    }
    println!();
    println!(
        "Running {} games per matchup × {} matchups = {} total",
        games_per_cell,
        VARIANTS.len() * VARIANTS.len(),
        total_games
    );
    println!();

    // Token-replay mode: if either TSOT_DECK_A_TOKEN or TSOT_DECK_B_TOKEN
    // is set, skip the full matchup sweep and play a single game with the
    // specified deck(s). When only one side is provided, the other falls
    // back to game_index=0 in the same matchup.
    let env_token_a = std::env::var("TSOT_DECK_A_TOKEN").ok();
    let env_token_b = std::env::var("TSOT_DECK_B_TOKEN").ok();
    let replay_mode = env_token_a.is_some() || env_token_b.is_some();

    let mut last_token_a = String::new();
    let mut last_token_b = String::new();

    if replay_mode {
        let token_a = env_token_a
            .as_deref()
            .map(|s| DeckToken::decode(s).expect("invalid TSOT_DECK_A_TOKEN"))
            .unwrap_or_else(|| DeckToken {
                master_seed: seed,
                side: Side::A,
                variant_a: DeckVariant::Ra,
                variant_b: DeckVariant::Ra,
                game_index: 0,
            });
        let token_b = env_token_b
            .as_deref()
            .map(|s| DeckToken::decode(s).expect("invalid TSOT_DECK_B_TOKEN"))
            .unwrap_or_else(|| DeckToken {
                master_seed: seed,
                side: Side::B,
                variant_a: token_a.variant_a,
                variant_b: token_a.variant_b,
                game_index: 0,
            });
        let v_a = token_a.variant_a;
        let v_b = token_b.variant_b;
        let pool_a = variant_pool(&playable_pool, v_a);
        let pool_b = variant_pool(&playable_pool, v_b);
        let mut rng_a = StdRng::seed_from_u64(token_a.per_deck_seed());
        let mut rng_b = StdRng::seed_from_u64(token_b.per_deck_seed());
        let deck_a = build_random_deck(&pool_a, &mut rng_a, 50, mandatory_for_variant(v_a));
        let deck_b = build_random_deck(&pool_b, &mut rng_b, 50, mandatory_for_variant(v_b));
        last_deck_a_ids = deck_a.iter().map(|c| c.id.clone()).collect();
        last_deck_b_ids = deck_b.iter().map(|c| c.id.clone()).collect();
        last_token_a = token_a.encode();
        last_token_b = token_b.encode();
        let deck_a_uniq: BTreeSet<String> = deck_a.iter().map(|c| c.id.clone()).collect();
        let deck_b_uniq: BTreeSet<String> = deck_b.iter().map(|c| c.id.clone()).collect();
        let state = GameState::new(deck_a, deck_b);
        let (mut stats, journal) = run_game(state, &mut rng, &mut last_log, registry.lua());
        stats.variant_a = v_a;
        stats.variant_b = v_b;
        stats.deck_a_ids = deck_a_uniq;
        stats.deck_b_ids = deck_b_uniq;
        stats.token_a = last_token_a.clone();
        stats.token_b = last_token_b.clone();
        stats.game_index = token_a.game_index;
        all.push(stats);
        last_journal = journal;
        println!("[replay] single-game token mode — A={last_token_a} B={last_token_b}");
    } else {
        for &v_a in &VARIANTS {
        for &v_b in &VARIANTS {
            let pool_a = &pools.iter().find(|(v, _)| *v == v_a).unwrap().1;
            let pool_b = &pools.iter().find(|(v, _)| *v == v_b).unwrap().1;
            for game_index in 0..games_per_cell {
                // Per-deck seeds derived from the (master_seed, side, v_a, v_b,
                // game_index) tuple. Each deck reproducible from its token alone.
                let token_a = DeckToken {
                    master_seed: seed,
                    side: Side::A,
                    variant_a: v_a,
                    variant_b: v_b,
                    game_index: game_index as u32,
                };
                let token_b = DeckToken {
                    master_seed: seed,
                    side: Side::B,
                    variant_a: v_a,
                    variant_b: v_b,
                    game_index: game_index as u32,
                };
                let mut rng_a = StdRng::seed_from_u64(token_a.per_deck_seed());
                let mut rng_b = StdRng::seed_from_u64(token_b.per_deck_seed());
                let deck_a =
                    build_random_deck(pool_a, &mut rng_a, 50, mandatory_for_variant(v_a));
                let deck_b =
                    build_random_deck(pool_b, &mut rng_b, 50, mandatory_for_variant(v_b));
                last_deck_a_ids = deck_a.iter().map(|c| c.id.clone()).collect();
                last_deck_b_ids = deck_b.iter().map(|c| c.id.clone()).collect();
                last_token_a = token_a.encode();
                last_token_b = token_b.encode();
                let deck_a_uniq: BTreeSet<String> =
                    deck_a.iter().map(|c| c.id.clone()).collect();
                let deck_b_uniq: BTreeSet<String> =
                    deck_b.iter().map(|c| c.id.clone()).collect();
                let state = GameState::new(deck_a, deck_b);
                last_log.clear();
                let (mut stats, journal) =
                    run_game(state, &mut rng, &mut last_log, registry.lua());
                stats.variant_a = v_a;
                stats.variant_b = v_b;
                stats.deck_a_ids = deck_a_uniq;
                stats.deck_b_ids = deck_b_uniq;
                stats.token_a = last_token_a.clone();
                stats.token_b = last_token_b.clone();
                stats.game_index = game_index as u32;
                all.push(stats);
                last_journal = journal;
            }
        }
        }
    }
    let elapsed = t0.elapsed();

    if let Some(path) = replay_out_path.as_ref() {
        let replay = tsot::replay::ReplayFile {
            seed,
            deck_a_card_ids: last_deck_a_ids.clone(),
            deck_b_card_ids: last_deck_b_ids.clone(),
            journal: last_journal,
        };
        match replay.to_json() {
            Ok(json) => match std::fs::write(path, &json) {
                Ok(()) => println!("[replay] wrote {} ({} bytes)", path, json.len()),
                Err(e) => eprintln!("[replay] failed to write {path}: {e}"),
            },
            Err(e) => eprintln!("[replay] failed to serialize: {e}"),
        }
    }

    println!();
    println!(
        "=== Last game: A={} B={} ===",
        last_token_a, last_token_b
    );
    print_deck_listing("Last game: A deck", &last_deck_a_ids);
    print_deck_listing("Last game: B deck", &last_deck_b_ids);
    println!("=== Last game: first 4 turns ===");
    for line in last_log.iter().take(4) {
        println!("  {line}");
    }
    println!();
    println!("=== Last game: last 4 turns ===");
    let start = last_log.len().saturating_sub(4);
    for line in &last_log[start..] {
        println!("  {line}");
    }

    print_aggregate(&all, elapsed);

    let report_path = std::env::var("TSOT_REPORT_OUT")
        .unwrap_or_else(|_| "tsot-report.html".to_string());
    if report_path != "-" {
        match report::write_html_report(&all, &pools, seed, elapsed, &report_path) {
            Ok(()) => println!("\n[report] wrote {report_path}"),
            Err(e) => eprintln!("[report] failed to write {report_path}: {e}"),
        }
    }

    Ok(())
}
