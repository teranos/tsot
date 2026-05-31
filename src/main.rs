mod champions_report;
mod report_style;
mod sim;

use clap::{Parser, Subcommand};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::path::Path;
use tsot::card::{Card, CardRegistry, CardType, CostSource};
use tsot::game::GameState;

use sim::{run_evolve, EvolveConfig};
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
    /// Round-robin matchup grid between evolved/baseline decks.
    MatchupEvolved(MatchupEvolvedArgs),
    /// Evolve a deck via genetic algorithm against a gauntlet.
    Evolve(EvolveArgs),
    /// Aggregate stats across a directory of saved champions.
    ChampionsReport(ChampionsReportArgs),
    /// For each baseline, evaluate champions in its Jaccard cluster
    /// against the current baselines, replace the baseline with the
    /// best live performer.
    CurateBaselines(CurateBaselinesArgs),
}

#[derive(Parser)]
struct CurateBaselinesArgs {
    /// Directory of champion candidates to consider for promotion.
    #[arg(long, default_value = "champions")]
    dir: String,
    /// Directory of baselines to upgrade in place.
    #[arg(long, default_value = "baselines")]
    baselines: String,
    /// Jaccard threshold for cluster membership (candidate is in
    /// baseline B's cluster if Jaccard(candidate, B) >= this).
    #[arg(long, default_value_t = 0.7)]
    threshold: f64,
    /// Games per side per (candidate, baseline) pairing during live
    /// re-evaluation. Total games per candidate = 2 × baselines × games.
    #[arg(long, default_value_t = 20)]
    games: u32,
    /// Master seed for the live evaluation RNG. Same seed → reproducible.
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
    seed: u64,
    /// Don't overwrite baseline files; print what would happen.
    #[arg(long = "dry-run")]
    dry_run: bool,
}

#[derive(Parser)]
struct MatchupEvolvedArgs {
    /// Directory containing EvolvedDeck JSON files to use as the
    /// players in the round-robin grid.
    #[arg(long, default_value = "baselines")]
    dir: String,
    /// Games per ordered (A, B) cell. With N decks, total games =
    /// N × N × this. Default 50 matches the variant matchup grid.
    #[arg(long, default_value_t = 50)]
    games: u32,
    /// Master seed for per-game RNG seeding. Same seed → byte-
    /// identical grid.
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
    seed: u64,
    /// Write an HTML grid report to this path.
    #[arg(long, value_name = "PATH", default_value = "matchup-evolved.html")]
    html: String,
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
    /// Jaccard-similarity threshold for clustering. Two decks are
    /// linked if `|A ∩ B| / |A ∪ B|` on card-id sets exceeds this.
    /// Single-linkage clustering then groups linked decks transitively.
    #[arg(long = "cluster-threshold", default_value_t = 0.7)]
    cluster_threshold: f64,
    /// Sample games per champion-vs-baseline pairing. Default 0 = skip
    /// game-level stats (fast, card-level only). With N > 0, each
    /// champion plays N games vs each baseline (both seats) and the
    /// report includes per-champion turn counts + action totals.
    #[arg(long = "sample-games", default_value_t = 0)]
    sample_games: u32,
    /// Directory of baseline opponents for the sample games. Default
    /// `baselines/`. Only used when `--sample-games > 0`.
    #[arg(long = "baselines", default_value = "baselines")]
    baselines: String,
    /// Seed for the sample-game RNG (for reproducibility).
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
    seed: u64,
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
    /// Master seed for every random decision in the run. Accepts
    /// decimal (`60104`) or hex (`0xEAC8`).
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
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

/// Parse a u64 from `--seed`, accepting hex (`0xEA03`) or decimal.
fn parse_u64_hex_or_dec(s: &str) -> Result<u64, std::num::ParseIntError> {
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(rest, 16)
    } else {
        s.parse::<u64>()
    }
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
/// For each baseline, find champions in its Jaccard cluster, live-
/// evaluate them against the current baselines, replace the baseline
/// with the highest-win-rate candidate. Apples-to-apples comparison
/// (every candidate fights the same opponent set), avoiding the
/// cross-round fitness bias of the saved-fitness curation path.
fn run_curate_baselines(
    registry: &CardRegistry,
    args: &CurateBaselinesArgs,
) -> mlua::Result<()> {
    use rand::Rng;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
        if a.is_empty() && b.is_empty() {
            return 1.0;
        }
        let inter = a.intersection(b).count() as f64;
        let union = a.union(b).count() as f64;
        if union > 0.0 { inter / union } else { 0.0 }
    }

    let baselines_dir = std::path::Path::new(&args.baselines);
    let champions_dir = std::path::Path::new(&args.dir);

    let mut baseline_paths: Vec<PathBuf> = match std::fs::read_dir(baselines_dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .map(|e| e.path())
            .collect(),
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", baselines_dir.display());
            std::process::exit(2);
        }
    };
    baseline_paths.sort();
    if baseline_paths.is_empty() {
        eprintln!("error: no baselines in {}", baselines_dir.display());
        std::process::exit(2);
    }

    let mut champion_paths: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(champions_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                champion_paths.push(p);
            }
        }
    }
    champion_paths.sort();

    // Materialize: (path, EvolvedDeck, Vec<Card>, id-set)
    let mut load = |path: &PathBuf| -> Option<(PathBuf, EvolvedDeck, Vec<Card>, BTreeSet<String>)> {
        let deck = EvolvedDeck::load(path).ok()?;
        let cards = deck.to_cards(registry).ok()?;
        let id_set: BTreeSet<String> = deck.card_ids.iter().cloned().collect();
        Some((path.clone(), deck, cards, id_set))
    };
    let baselines: Vec<_> = baseline_paths.iter().filter_map(&mut load).collect();
    let champions: Vec<_> = champion_paths.iter().filter_map(&mut load).collect();

    println!(
        "Live-curate: {} baselines × {} champions, threshold {:.2}, {} games/side per pairing",
        baselines.len(),
        champions.len(),
        args.threshold,
        args.games
    );

    // Snapshot baseline decks for evaluation — all candidates fight
    // the same opponent set regardless of in-flight upgrades.
    let baseline_decks: Vec<Vec<Card>> = baselines.iter().map(|(_, _, c, _)| c.clone()).collect();
    let baseline_labels: Vec<String> = baselines.iter().map(|(_, d, _, _)| d.label.clone()).collect();

    let mut rng = StdRng::seed_from_u64(args.seed);

    let evaluate = |cand_cards: &[Card], rng: &mut StdRng| -> f64 {
        let mut wins = 0u32;
        let mut games = 0u32;
        for opp in &baseline_decks {
            for _ in 0..args.games {
                // candidate as A
                let state = GameState::new(cand_cards.to_vec(), opp.clone());
                let mut game_rng = StdRng::seed_from_u64(rng.gen());
                let mut log: Vec<String> = Vec::new();
                let (stats, _) = sim::run_game(state, &mut game_rng, &mut log, registry.lua());
                if stats.winner == tsot::game::PlayerId::A {
                    wins += 1;
                }
                games += 1;
                // candidate as B
                let state = GameState::new(opp.clone(), cand_cards.to_vec());
                let mut game_rng = StdRng::seed_from_u64(rng.gen());
                let mut log = Vec::new();
                let (stats, _) = sim::run_game(state, &mut game_rng, &mut log, registry.lua());
                if stats.winner == tsot::game::PlayerId::B {
                    wins += 1;
                }
                games += 1;
            }
        }
        wins as f64 / games as f64
    };

    let mut changes = 0u32;
    let mut all_matched: BTreeSet<PathBuf> = BTreeSet::new();
    for (bidx, (bpath, bdata, _bcards, bset)) in baselines.iter().enumerate() {
        // Cluster: this baseline plus any champion >= threshold Jaccard.
        let mut cluster: Vec<(PathBuf, &EvolvedDeck, &Vec<Card>)> = Vec::new();
        cluster.push((bpath.clone(), bdata, &baselines[bidx].2));
        for (cpath, cdata, ccards, cset) in &champions {
            let jacc = jaccard(bset, cset);
            if jacc >= args.threshold {
                cluster.push((cpath.clone(), cdata, ccards));
                all_matched.insert(cpath.clone());
            }
        }
        println!();
        println!(
            "Cluster for {} ({} members):",
            bpath.file_name().unwrap().to_string_lossy(),
            cluster.len()
        );
        let mut scored: Vec<(PathBuf, f64, f64)> = Vec::new();
        for (cpath, cdata, ccards) in &cluster {
            let live = evaluate(ccards, &mut rng);
            println!(
                "  {:<40}  prior={:.3}  live={:.3}",
                cpath.file_name().unwrap().to_string_lossy(),
                cdata.fitness,
                live
            );
            scored.push((cpath.clone(), live, cdata.fitness));
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (winner_path, winner_live, _) = scored[0].clone();
        if winner_path == *bpath {
            println!(
                "  → keep {} (already best in cluster, live={:.3})",
                bpath.file_name().unwrap().to_string_lossy(),
                winner_live
            );
        } else {
            // Load winner's saved deck and write it to baseline's path
            // (preserving the baseline's filename — it's the stable handle).
            match EvolvedDeck::load(&winner_path) {
                Ok(mut new_deck) => {
                    new_deck.fitness = winner_live;
                    new_deck.label = format!("{}_curated", new_deck.label);
                    if args.dry_run {
                        println!(
                            "  → would upgrade {} ← {} (live={:.3}) [dry-run]",
                            bpath.file_name().unwrap().to_string_lossy(),
                            winner_path.file_name().unwrap().to_string_lossy(),
                            winner_live
                        );
                    } else {
                        match new_deck.save(bpath) {
                            Ok(()) => {
                                println!(
                                    "  → UPGRADED {} ← {} (live={:.3})",
                                    bpath.file_name().unwrap().to_string_lossy(),
                                    winner_path.file_name().unwrap().to_string_lossy(),
                                    winner_live
                                );
                                changes += 1;
                            }
                            Err(e) => eprintln!("  ! save failed: {e}"),
                        }
                    }
                }
                Err(e) => eprintln!("  ! reload of {} failed: {e}", winner_path.display()),
            }
        }
    }
    println!();
    println!(
        "Done: {} baseline(s) upgraded, {} champions matched to a cluster, {} unmatched",
        changes,
        all_matched.len(),
        champions.len() - all_matched.len()
    );
    let unmatched: Vec<&PathBuf> = champions
        .iter()
        .map(|(p, _, _, _)| p)
        .filter(|p| !all_matched.contains(*p))
        .collect();
    if !unmatched.is_empty() {
        println!("Unmatched champions (potential new attractors):");
        for p in unmatched {
            println!("  {}", p.display());
        }
    }
    let _ = baseline_labels;
    Ok(())
}

/// Round-robin grid of evolved decks against each other. Loads every
/// `*.json` file from `args.dir`, plays each ordered (A, B) pair for
/// `args.games` games, prints the win-rate matrix + per-deck overall
/// win rate. Deterministic per `args.seed`.
fn run_matchup_evolved(
    registry: &CardRegistry,
    args: &MatchupEvolvedArgs,
) -> mlua::Result<()> {
    use rand::Rng;
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
    let mut decks: Vec<Vec<tsot::card::Card>> = Vec::new();
    for path in &paths {
        match EvolvedDeck::load(path) {
            Ok(saved) => match saved.to_cards(registry) {
                Ok(cards) => {
                    labels.push(saved.label.clone());
                    decks.push(cards);
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
    // Parallel index: which (a_idx, b_idx) produced each entry in
    // all_stats — used for per-deck aggregation later.
    let mut game_keys: Vec<(usize, usize)> = Vec::with_capacity(n * n * args.games as usize);
    let t0 = std::time::Instant::now();
    let mut rng = StdRng::seed_from_u64(args.seed);
    for i in 0..n {
        for j in 0..n {
            for _ in 0..args.games {
                let state = GameState::new(decks[i].clone(), decks[j].clone());
                let game_seed: u64 = rng.gen();
                let mut game_rng = StdRng::seed_from_u64(game_seed);
                let mut log: Vec<String> = Vec::new();
                let (stats, _) = sim::run_game(state, &mut game_rng, &mut log, registry.lua());
                if stats.winner == tsot::game::PlayerId::A {
                    wins[i][j] += 1;
                }
                all_stats.push(stats);
                game_keys.push((i, j));
            }
        }
    }
    let elapsed = t0.elapsed();

    // Aggregate game-level metrics across all cells.
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
    // Per-deck turn count: mean turn count of all games where this deck
    // participated (either seat, excluding mirror-self matches).
    let mut per_deck_turns: Vec<(u32, u32, u32, u32)> = vec![(0, 0, u32::MAX, 0); n]; // (sum, count, min, max)
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
    println!("  {:<w$}  {:>8}  {:>8}  {:>8}  {:>8}", "deck", "min", "mean", "median", "max", w = label_w);
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
        "choose_int", "confirm",
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

    // Per-card performance: total plays across all games (both sides
    // combined), mean turn first played. Helps spot which cards are
    // load-bearing vs filler in the deck pool.
    let mut card_play_totals: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let mut card_first_turn_sum: std::collections::BTreeMap<String, (u32, u32)> =
        std::collections::BTreeMap::new(); // (turn_sum, count)
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

    // Interesting games — three picks: shortest, longest, biggest mill
    // imbalance (most one-sided defender-decking).
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
    // Print the win-rate matrix.
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

    // Per-deck overall record: wins as A across all opponents + wins
    // when others played A against this deck (= 1 - their cell rate).
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

    // HTML grid.
    let html_path = &args.html;
    match write_matchup_evolved_html(
        &labels,
        &wins,
        args.games,
        &args.dir,
        html_path,
        &all_stats,
        &game_keys,
    ) {
        Ok(()) => println!("HTML grid written to {html_path}"),
        Err(e) => eprintln!("failed to write HTML to {html_path}: {e}"),
    }
    Ok(())
}

fn write_matchup_evolved_html(
    labels: &[String],
    wins: &[Vec<u32>],
    games: u32,
    dir: &str,
    path: &str,
    all_stats: &[sim::GameStats],
    game_keys: &[(usize, usize)],
) -> std::io::Result<()> {
    use maud::{html, PreEscaped, DOCTYPE};
    let n = labels.len();
    fn rate_color(r: f64) -> String {
        let t = r.clamp(0.0, 1.0);
        let red = ((1.0 - t) * 100.0 + 30.0) as u8;
        let green = (t * 100.0 + 30.0) as u8;
        format!("background: rgb({red},{green},40); color: #eee;")
    }

    // Aggregate metrics across all games.
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

    // Per-deck turn stats (mean, min, median, max) when this deck plays.
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
        "choose_int", "confirm",
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

    // Top-N played cards across all games.
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

    // Interesting games (shortest, longest, rout).
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

    // Pre-compute row averages and per-deck both-seat win rates so the
    // maud template can stay declarative.
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
                                td { (cid) }
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

/// Per-champion game-level aggregate from `--sample-games`. Empty
/// `turns` means no sample run was done.
#[derive(Default, Clone)]
pub(crate) struct ChampGameStats {
    pub turns: Vec<u32>,
    pub attacks: u64,
    pub deaths: u64,
    pub milled: u64,
    pub played: u64,
}

/// Aggregate card-level signal across all saved EvolvedDeck files in a
/// directory. Prints frequency, mean copies, and fitness correlation —
/// which cards consistently survive selection across many runs.
fn run_champions_report(
    registry: &CardRegistry,
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

    // Clustering: union-find with Jaccard threshold on card-id sets.
    // Single-linkage — if pair (i, j) has Jaccard > threshold, i and j
    // share a cluster transitively. Useful for spotting same-attractor
    // groups (e.g., r3-rank1..5 cluster, eac8 champions cluster).
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
            let inter = champ_sets[i].intersection(&champ_sets[j]).count() as f64;
            let union = champ_sets[i].union(&champ_sets[j]).count() as f64;
            let jacc = if union > 0.0 { inter / union } else { 0.0 };
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

    // Optional game-level sampling: each champion plays N games vs each
    // baseline (both seats). Aggregated to per-champion turn count +
    // action totals so champions-report can answer "which champion runs
    // fast games?" / "which mills the most?" — questions card-frequency
    // alone can't.
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
                            // Champion's "side" depends on swap.
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
            println!(
                "Done sampling in {:.2?}",
                t_sample.elapsed()
            );
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
    println!();

    // Gauntlet: load curated EA-evolved decks from `baselines/`. These
    // replaced the older random variant decks (ra/rb/hu/go/uu/pr/gg —
    // built fresh per run from variant_pool). The baselines are real
    // evolved attractors picked for diversity, so the EA always fights
    // strong known-good decks, not random samples. --no-variants skips
    // baselines too (gauntlet then = only --extra files).
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
        paths.sort(); // deterministic load order
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
        let baseline_count = if no_variants { 0 } else { gauntlet.len() - args.extras.len() };
        println!(
            "Gauntlet now {} decks total ({} baselines + {} extras)",
            gauntlet.len(),
            baseline_count,
            args.extras.len(),
        );
    }
    if gauntlet.is_empty() {
        eprintln!(
            "error: gauntlet is empty — either populate baselines/ or pass --extra PATH when --no-variants is set."
        );
        std::process::exit(2);
    }
    // Budget print uses the actual gauntlet size after baselines + extras.
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
        return run_champions_report(&registry, &playable_pool, args);
    }
    if let Some(Command::MatchupEvolved(args)) = &cli.command {
        return run_matchup_evolved(&registry, args);
    }
    if let Some(Command::CurateBaselines(args)) = &cli.command {
        return run_curate_baselines(&registry, args);
    }

    // No subcommand: print help-style hint and exit non-zero. The
    // legacy 7×7 variant matchup mode was removed — every analytic it
    // produced now lives in `tsot matchup-evolved` / `tsot champions-report`
    // against curated baselines instead of randomly-sampled variant decks.
    eprintln!("no subcommand specified. run with --help to see the available commands.");
    std::process::exit(2);
}
