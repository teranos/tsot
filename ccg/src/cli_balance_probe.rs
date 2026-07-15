//! `tsot balance-probe` subcommand: side-by-side comparison of card
//! variants. Authors declare variants inline in the card's .lua via a
//! `variants = { [key] = { overrides } }` table; the registry loads
//! each variant as a hidden card with id `{base-id}-{key}`. This
//! subcommand picks them up automatically. No file paths, no flag
//! juggling — the LLM edits the .lua, you run `make probe`.
//!
//! Variant schema example (in `cards/dark-salamander.lua`):
//!
//! ```lua
//! return {
//!   id = "dark-salamander",
//!   -- ... base definition ...
//!   variants = {
//!     ["1pwr"] = { activated = {...} },
//!     ["2pwr"] = { activated = {...} },
//!   },
//! }
//! ```
//!
//! Commands: `make probe` auto-discovers every card with declared
//! variants and probes each (base + variants). `make probe
//! dark-salamander` probes just that card. For each card under probe,
//! runs a short EA with the base / each variant pinned into every
//! genome, reports the ceiling fitness and top co-occurring cards.
//! Variants are excluded from the main `playable_pool` so they don't
//! pollute `make evolve`.
//!
//! Quick-triage defaults: pop=12, gens=8, n=10 — ~15-25s per
//! variant on 8 cores, noise floor σ ≈ 0.043. For balance-gating
//! decisions where you need to distinguish subtler differences,
//! `make probe-long` runs pop=30 gens=15 n=30 (~3 min per variant,
//! σ ≈ 0.025).

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser;
use maud::{html, Markup, PreEscaped, DOCTYPE};

use tsot::card::{Card, CardRegistry};

use crate::parse_u64_hex_or_dec;
use crate::report_style;
use tsot::sim::evolved_deck::EvolvedDeck;
use tsot::sim::{run_evolve, EvolveConfig};

#[derive(Parser)]
pub struct BalanceProbeArgs {
    /// Card ids to probe. Each id expands to the base card + every
    /// variant declared in its .lua file's `variants` table. With no
    /// ids passed, every card in the registry that declares at least
    /// one variant is probed automatically.
    #[arg(value_name = "CARD_ID")]
    pub card_ids: Vec<String>,
    /// Copies of the variant card to pin into every genome.
    #[arg(long = "pinned-count", default_value_t = 2)]
    pub pinned_count: usize,
    /// Population size. Default 8 — daily-fast budget. Bump for
    /// tighter ceiling search.
    #[arg(long, default_value_t = 8)]
    pub pop: usize,
    /// Generations. Default 4 — daily-fast budget. Bump for deeper
    /// search.
    #[arg(long, default_value_t = 4)]
    pub gens: usize,
    /// Games per side per fitness eval. Default 3 — paired with UCT
    /// opponent for higher per-game signal; heuristic-vs-heuristic
    /// needed 10+ for the same separation.
    #[arg(long, default_value_t = 3)]
    pub n: u32,
    /// Master seed.
    #[arg(long, default_value_t = 0xBA_1A, value_parser = parse_u64_hex_or_dec)]
    pub seed: u64,
    /// Tournament size for selection.
    #[arg(long = "tournament-k", default_value_t = 3)]
    pub tournament_k: usize,
    /// Per-slot mutation probability.
    #[arg(long, default_value_t = 0.03)]
    pub rate: f64,
    /// Elite carryover count.
    #[arg(long, default_value_t = 1)]
    pub elite: usize,
    /// Baselines directory.
    #[arg(long, default_value = "baselines")]
    pub baselines: String,
    /// Output HTML report path. Use `-` to skip.
    #[arg(long = "html-report", default_value = "balance-probe-report.html")]
    pub html_report: String,
    /// Output JSON path prefix. Per-variant JSONs go to
    /// `{prefix}-{variant_id}.json`. Use `-` to skip JSON output.
    #[arg(long = "json-prefix", default_value = "balance-probe")]
    pub json_prefix: String,
    /// Opponent AI for fitness evaluation. Default `uct` (UCB1
    /// tree-search MCTS — stronger play, the variant deltas mean
    /// more). `game` is the fast baseline (alias: `heuristic`). Candidate side
    /// stays Heuristic regardless. Mirrors `tsot evolve`'s flag.
    #[arg(long = "opponent-ai", default_value = "uct")]
    pub opponent_ai: String,
    /// UCT iterations per pick when `--opponent-ai uct`. Default 10
    /// is the fast probe setting (~3× heuristic cost per game, still
    /// strong-enough lookahead for the per-card signal to read).
    /// Bump to 50 for the matched-compute-to-one-ply-MCTS sweet spot
    /// when you want tighter σ.
    #[arg(long = "opponent-uct-iterations", default_value_t = 10)]
    pub opponent_uct_iterations: u32,
    /// UCT exploration constant when `--opponent-ai uct`. `sqrt(2)` is
    /// classical.
    #[arg(long = "opponent-uct-c", default_value_t = std::f64::consts::SQRT_2)]
    pub opponent_uct_c: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ProbeResult {
    /// Probe id — the card.id of the version being probed (base id
    /// or `{base}-{variant}` for variants).
    card_id: String,
    /// For variants, the base card's id; None for base / non-variant.
    variant_of: Option<String>,
    seed: u64,
    pop_size: usize,
    generations_run: usize,
    n_per_side: u32,
    pinned_count: usize,
    final_best_fitness: f64,
    final_mean_fitness: f64,
    final_best_genome: Vec<String>,
    top_genome_card_counts: BTreeMap<String, u32>,
    final_pop_card_presence: BTreeMap<String, f64>,
    best_fitness_curve: Vec<f64>,
    mean_fitness_curve: Vec<f64>,
}

fn load_baselines(registry: &CardRegistry, dir: &str) -> (Vec<Vec<Card>>, Vec<String>) {
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .map(|e| e.path())
            .collect(),
        Err(e) => {
            eprintln!("error: cannot read baselines dir {dir}: {e}");
            std::process::exit(2);
        }
    };
    paths.sort();
    let mut decks: Vec<Vec<Card>> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for p in &paths {
        match EvolvedDeck::load(p) {
            Ok(saved) => match saved.to_cards(registry) {
                Ok(cards) => {
                    labels.push(saved.label.clone());
                    decks.push(cards);
                }
                Err(e) => eprintln!("  ! baseline {} unloadable: {e}", p.display()),
            },
            Err(e) => eprintln!("  ! baseline {} unparseable: {e}", p.display()),
        }
    }
    (decks, labels)
}

/// Expand a base card id to (base, [variants...]) using the
/// registry's `variant_of` chain. Variants are cards with
/// `variant_of = Some(base_id)`.
fn expand_to_base_and_variants<'a>(
    registry: &'a CardRegistry,
    base_id: &str,
) -> Vec<&'a Card> {
    let mut out: Vec<&Card> = Vec::new();
    if let Some(base) = registry.get(base_id) {
        out.push(base);
    }
    for c in registry.cards() {
        if c.variant_of.as_deref() == Some(base_id) {
            out.push(c);
        }
    }
    out
}

/// Auto-discovery: every base card that has at least one variant.
/// A base is any non-variant card that appears as some other card's
/// `variant_of`. Returns base ids in sorted order so the report is
/// deterministic.
fn discover_base_ids_with_variants(registry: &CardRegistry) -> Vec<String> {
    let mut bases = std::collections::BTreeSet::<String>::new();
    for c in registry.cards() {
        if let Some(base_id) = &c.variant_of {
            bases.insert(base_id.clone());
        }
    }
    bases.into_iter().collect()
}

fn probe_one_card(
    registry: &std::sync::Arc<CardRegistry>,
    pool: &[Card],
    gauntlet: &[Vec<Card>],
    args: &BalanceProbeArgs,
    card: &Card,
) -> ProbeResult {
    let opponent_ai = match args.opponent_ai.to_ascii_lowercase().as_str() {
        "game" | "heuristic" => tsot::sim::AiKind::Game,
        "uct" => tsot::sim::AiKind::Uct(tsot::sim::uct::UctConfig {
            iterations: args.opponent_uct_iterations,
            exploration_c: args.opponent_uct_c,
            ..Default::default()
        }),
        other => {
            eprintln!("error: --opponent-ai must be 'game' | 'uct' ('heuristic' accepted as legacy alias), got {other:?}");
            std::process::exit(2);
        }
    };
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
        stop_at_ceiling: None,
        stop_at_plateau: None,
        plateau_epsilon: 0.0,
        pinned_card_id: Some(card.id.clone()),
        pinned_count: args.pinned_count.min(3),
        diversity_alpha: 0.0,
        opponent_ai,
    };

    // For pin to work, the pinned card MUST be available to genomes.
    // Variants are excluded from `playable_pool` by design, so when we
    // probe a variant we extend the pool with that one variant card.
    let pool_with_pin: Vec<Card> = if card.is_variant {
        let mut p = pool.to_vec();
        p.push(card.clone());
        p
    } else {
        pool.to_vec()
    };

    let t_start = std::time::Instant::now();
    let mut t_prev = t_start;
    let total_gens = cfg.generations;
    let mut prev_best: Option<f64> = None;
    let card_id_for_log = card.id.clone();
    let result = {
        let cb = &mut |gen: usize, p: &[(Vec<String>, f64)]| {
            let now = std::time::Instant::now();
            let took = now - t_prev;
            let total = now - t_start;
            let best = p[0].1;
            let mean: f64 = p.iter().map(|(_, f)| *f).sum::<f64>() / p.len() as f64;
            let new_best = match prev_best {
                Some(prev) if best > prev + f64::EPSILON => " | NEW BEST",
                _ => "",
            };
            prev_best = Some(best);
            println!(
                "  [{card_id_for_log}] gen {gen:>2}/{total_gens} | best={best:.3} mean={mean:.3} | took {took:>5.1?} | total {total:>5.1?}{new_best}"
            );
            t_prev = now;
        };
        run_evolve(registry, &pool_with_pin, gauntlet, &cfg, cb)
    };

    let final_best = &result.final_population[0];
    let final_best_fitness = final_best.1;
    let final_mean_fitness: f64 = result.final_population.iter().map(|(_, f)| *f).sum::<f64>()
        / result.final_population.len() as f64;
    let mut top_counts: BTreeMap<String, u32> = BTreeMap::new();
    for id in &final_best.0 {
        *top_counts.entry(id.clone()).or_insert(0) += 1;
    }
    let pop_size = result.final_population.len() as f64;
    let mut presence: BTreeMap<String, f64> = BTreeMap::new();
    for (genome, _) in &result.final_population {
        let unique: std::collections::BTreeSet<&str> = genome.iter().map(|s| s.as_str()).collect();
        for id in unique {
            *presence.entry(id.to_string()).or_insert(0.0) += 1.0;
        }
    }
    for v in presence.values_mut() {
        *v /= pop_size;
    }
    let best_curve: Vec<f64> = result.best_per_generation.iter().map(|(_, f)| *f).collect();
    let mean_curve = result.per_gen_mean_fitness.clone();
    let gens_run = result.best_per_generation.len().saturating_sub(1);

    ProbeResult {
        card_id: card.id.clone(),
        variant_of: card.variant_of.clone(),
        seed: args.seed,
        pop_size: args.pop,
        generations_run: gens_run,
        n_per_side: args.n,
        pinned_count: args.pinned_count.min(3),
        final_best_fitness,
        final_mean_fitness,
        final_best_genome: final_best.0.clone(),
        top_genome_card_counts: top_counts,
        final_pop_card_presence: presence,
        best_fitness_curve: best_curve,
        mean_fitness_curve: mean_curve,
    }
}

/// Render the probed card's full detail (cost, stats, abilities,
/// flavor) as a hero panel at the top of its variant section. So you
/// can see the card without opening the .lua.
fn variant_hero(card: &Card) -> Markup {
    use tsot::card::CostSource;
    let color_class = if card.frame.as_deref() == Some("transparent") {
        "ci-transparent"
    } else if card.face.iter().any(|f| f == "glow") {
        "ci-glow"
    } else {
        match card.colors.first().map(String::as_str) {
            Some("red") => "ci-red",
            Some("blue") => "ci-blue",
            Some("green") => "ci-green",
            Some("purple") => "ci-purple",
            Some("black") => "ci-black",
            Some("white") => "ci-white",
            Some("pink") => "ci-pink",
            Some("orange") => "ci-orange",
            Some("azure") => "ci-azure",
            _ => "ci-colorless",
        }
    };
    let display_name = if card.name.is_empty() {
        card.id.clone()
    } else {
        card.name.clone()
    };
    let kind = match card.kind {
        tsot::card::CardType::Creature => "creature",
        tsot::card::CardType::Spell => match card.timing {
            Some(tsot::card::Timing::Instant) => "instant",
            Some(tsot::card::Timing::Sorcery) => "sorcery",
            None => "spell",
        },
        tsot::card::CardType::Artifact => "artifact",
        tsot::card::CardType::Environment => "environment",
        tsot::card::CardType::Mutation => "mutation",
        tsot::card::CardType::Symbol => "symbol",
        tsot::card::CardType::Unspecified => "card",
    };
    let cost_str = if card.cost.is_empty() {
        "free".to_string()
    } else {
        card.cost
            .iter()
            .map(|c| {
                let amt = if c.is_x { "X".to_string() } else { c.amount.to_string() };
                let source = match c.source {
                    CostSource::Hand => "hand",
                    CostSource::Mill => "mill",
                    CostSource::Graveyard => "graveyard",
                    CostSource::Sacrifice => "sacrifice",
                    CostSource::SelfExile => "self-exile",
                    CostSource::Attached => "attached",
                };
                format!("{amt} {source}")
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let stats = card.stats.map(|s| format!("{}/{}", s.x, s.y));
    let meta_parts: Vec<String> = {
        let mut parts: Vec<String> = Vec::new();
        if !card.colors.is_empty() {
            parts.push(card.colors.join(" "));
        }
        parts.push(kind.to_string());
        if !card.subtypes.is_empty() {
            parts.push(format!("— {}", card.subtypes.join("/")));
        }
        parts
    };
    let symbols_str = if card.symbols.is_empty() {
        None
    } else {
        Some(card.symbols.join(" "))
    };
    html! {
        div.variant-hero {
            div.vh-head {
                span class={ "ci-color " (color_class) } {}
                span.vh-name { (display_name) }
                @if let Some(sym) = symbols_str { span.vh-symbols { (sym) } }
                @if let Some(s) = stats { span.vh-stats { (s) } }
            }
            div.vh-meta { (meta_parts.join(" ")) }
            div.vh-cost { "cost: " (cost_str) }
            @if !card.abilities.is_empty() {
                div.vh-abilities {
                    @for a in &card.abilities { div { (a) } }
                }
            }
            @if !card.flavor.is_empty() {
                div.vh-flavor { (card.flavor) }
            }
        }
    }
}

fn build_comparison_html(
    args: &BalanceProbeArgs,
    pool: &[Card],
    groups: &[(String, Vec<(Card, ProbeResult)>)],
) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "tsot — balance probe" }
                style { (PreEscaped(report_style::CSS)) }
                style { "
                    .group-card { border: 1px solid var(--border); border-left: 4px solid var(--accent);
                                  padding: 10px 14px; margin: 0 0 1.5em; background: var(--bg-panel); }
                    .group-card h2 { margin: 0 0 0.6em; color: var(--text-emphasis); font-size: 16px; }
                    .variant-card { border: 1px solid var(--border); border-left: 3px solid var(--accent);
                                    padding: 8px 12px; margin: 0 0 1em; background: var(--bg-panel); }
                    .variant-card h3 { margin: 0 0 0.4em; color: var(--text-emphasis); font-size: 14px; }
                    .variant-card .stats { display: flex; gap: 16px; font-size: 12px; margin-bottom: 8px; }
                    .variant-card .stats b { color: var(--text-emphasis); }
                    .winner { border-left-color: #60c870; }
                    .winner h3::after { content: \" — strongest\"; color: #60c870; font-size: 11px; }
                    .variant-hero { background: var(--bg); border: 1px solid var(--border-soft);
                                    padding: 8px 10px; margin: 0 0 8px; font-size: 12px; }
                    .variant-hero .vh-head { display: flex; align-items: center; gap: 8px;
                                             margin-bottom: 3px; }
                    .variant-hero .vh-name { font-weight: 600; color: var(--text-emphasis); }
                    .variant-hero .vh-stats { margin-left: auto; color: var(--accent); font-weight: 600; }
                    .variant-hero .vh-symbols { color: var(--text-tertiary); font-size: 13px; }
                    .variant-hero .vh-meta { color: var(--text-secondary); font-size: 11px; }
                    .variant-hero .vh-cost { color: var(--text-secondary); margin: 2px 0; }
                    .variant-hero .vh-abilities { margin-top: 4px; color: var(--text); }
                    .variant-hero .vh-abilities > div { font-size: 11px; padding: 1px 0; }
                    .variant-hero .vh-flavor { color: var(--text-tertiary); font-style: italic;
                                               font-size: 11px; margin-top: 4px; }
                " }
            }
            body {
                h1 { "tsot — balance probe" }
                div.meta {
                    div { span.k { "groups" } b { (groups.len()) } }
                    div { span.k { "pinned copies" } b { (args.pinned_count) } }
                    div { span.k { "pop" } b { (args.pop) } }
                    div { span.k { "gens" } b { (args.gens) } }
                    div { span.k { "n/side" } b { (args.n) } }
                    div { span.k { "seed" } b { (format!("{:#x}", args.seed)) } }
                }

                p.note {
                    "Each variant runs an EA with itself pinned to "
                    (args.pinned_count) " copies in every genome. The reported "
                    "fitness is the ceiling the EA reaches when forced to build "
                    "around that variant. Higher ceiling = stronger variant. The "
                    "co-occurring cards show the archetype each variant slots into. "
                    "Add variants by declaring `variants = { [key] = { overrides } }` "
                    "in the card's .lua file."
                }

                @for (base_id, members) in groups {
                    @let max_best = members.iter()
                        .map(|(_, r)| r.final_best_fitness)
                        .fold(f64::NEG_INFINITY, f64::max);
                    div.group-card {
                        h2 { (base_id) " — " (members.len()) " version"
                             (if members.len() == 1 { "" } else { "s" }) }
                        @for (card, r) in members {
                            @let is_winner = (r.final_best_fitness - max_best).abs() < 1e-9
                                && members.len() > 1;
                            div class=(if is_winner { "variant-card winner" } else { "variant-card" }) {
                                h3 { (r.card_id) }
                                (variant_hero(card))
                                div.stats {
                                    div { "best fitness: " b { (format!("{:.3}", r.final_best_fitness)) } }
                                    div { "mean fitness: " b { (format!("{:.3}", r.final_mean_fitness)) } }
                                    div { "gens run: " b { (r.generations_run) } "/" (r.pop_size) }
                                }
                                div {
                                    b { "Top final deck — co-occurring cards (count × card):" }
                                    div.mini-card-row {
                                        @for (cid, n) in r.top_genome_card_counts.iter().filter(|(id, _)| id.as_str() != r.card_id.as_str()) {
                                            (report_style::mini_card(pool, cid, *n as usize, 50))
                                        }
                                    }
                                }
                                details {
                                    summary { "fitness curve (per generation)" }
                                    pre style="font-size: 10px;" {
                                        ("gen  best   mean\n".to_string())
                                        @for i in 0..r.best_fitness_curve.len() {
                                            (format!("{:>3}  {:.3}  {:.3}\n",
                                                i,
                                                r.best_fitness_curve[i],
                                                r.mean_fitness_curve.get(i).copied().unwrap_or(0.0),
                                            ))
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

pub fn run_balance_probe(
    registry: &std::sync::Arc<CardRegistry>,
    playable_pool: &[Card],
    args: &BalanceProbeArgs,
) -> mlua::Result<()> {
    // Decide which base ids to probe. No args → auto-discover every
    // base that has at least one variant declared.
    let base_ids: Vec<String> = if args.card_ids.is_empty() {
        let discovered = discover_base_ids_with_variants(registry);
        if discovered.is_empty() {
            eprintln!(
                "No cards with variants declared in `cards/`. Add a `variants = {{ [key] = {{ ... }} }}` block to a card's .lua, or pass card ids as positional arguments."
            );
            std::process::exit(2);
        }
        println!(
            "Auto-discovered {} card(s) with variants: {}",
            discovered.len(),
            discovered.join(", ")
        );
        discovered
    } else {
        // Validate that each requested id resolves to either a base
        // card or a card with variants.
        let mut missing: Vec<String> = Vec::new();
        for id in &args.card_ids {
            if registry.get(id).is_none() {
                missing.push(id.clone());
            }
        }
        if !missing.is_empty() {
            eprintln!("error: unknown card id(s): {}", missing.join(", "));
            std::process::exit(2);
        }
        args.card_ids.clone()
    };

    let (gauntlet, gauntlet_labels) = load_baselines(registry, &args.baselines);
    if gauntlet.is_empty() {
        eprintln!(
            "error: gauntlet is empty — populate {} with baseline JSONs.",
            args.baselines
        );
        std::process::exit(2);
    }

    println!();
    println!("=== balance-probe ===");
    println!("  probing: {}", base_ids.join(", "));
    println!(
        "  pinned_count={} pop={} gens={} n={} seed={:#x}",
        args.pinned_count, args.pop, args.gens, args.n, args.seed,
    );
    println!("  baselines: {} decks loaded", gauntlet.len());
    for (label, deck) in gauntlet_labels.iter().zip(&gauntlet) {
        println!("    + {label} ({} cards)", deck.len());
    }
    println!();

    let mut groups: Vec<(String, Vec<(Card, ProbeResult)>)> = Vec::new();
    for base_id in &base_ids {
        let group_members = expand_to_base_and_variants(registry, base_id);
        if group_members.is_empty() {
            eprintln!("warn: no cards found for base id {base_id}");
            continue;
        }
        println!("--- group: {base_id} ({} version(s)) ---", group_members.len());
        let mut group_results: Vec<(Card, ProbeResult)> = Vec::new();
        for card in &group_members {
            println!("--- probing {} ---", card.id);
            let result = probe_one_card(registry, playable_pool, &gauntlet, args, card);
            if args.json_prefix != "-" {
                let path = format!("{}-{}.json", args.json_prefix, card.id);
                match serde_json::to_string_pretty(&result)
                    .map_err(|e| mlua::Error::runtime(format!("serialize {}: {e}", card.id)))
                    .and_then(|s| {
                        std::fs::write(&path, s)
                            .map_err(|e| mlua::Error::runtime(format!("write {path}: {e}")))
                    }) {
                    Ok(()) => println!("  → wrote {path}"),
                    Err(e) => eprintln!("  ! failed to write {path}: {e}"),
                }
            }
            group_results.push(((*card).clone(), result));
            println!();
        }
        groups.push((base_id.clone(), group_results));
    }

    // Summary table, grouped by base id.
    println!("=== summary ===");
    for (base_id, members) in &groups {
        println!();
        println!("--- {base_id} ---");
        let name_width = members
            .iter()
            .map(|(_, r)| r.card_id.len())
            .max()
            .unwrap_or(10)
            .max(10);
        for (_, r) in members {
            let mut coocc: Vec<(&str, u32)> = r
                .top_genome_card_counts
                .iter()
                .filter(|(id, _)| id.as_str() != r.card_id.as_str())
                .map(|(id, n)| (id.as_str(), *n))
                .collect();
            coocc.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            let top: Vec<String> = coocc.iter().take(5).map(|(id, n)| format!("{n}×{id}")).collect();
            println!(
                "  {:<width$}  best={:.3}  mean={:.3}  | top: {}",
                r.card_id,
                r.final_best_fitness,
                r.final_mean_fitness,
                top.join(", "),
                width = name_width,
            );
        }
    }

    if args.html_report != "-" {
        let markup = build_comparison_html(args, registry.cards(), &groups);
        match std::fs::write(&args.html_report, markup.into_string()) {
            Ok(()) => {
                println!();
                println!("→ wrote HTML report: {}", args.html_report);
            }
            Err(e) => eprintln!("! failed to write HTML report: {e}"),
        }
    }

    Ok(())
}
