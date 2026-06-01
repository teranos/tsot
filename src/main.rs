mod champions_report;
mod cli_balance_probe;
mod cli_champions_report;
mod cli_curate;
mod cli_curve_sample;
mod cli_evolve;
mod cli_matchup_evolved;
mod cli_prune_champions;
mod evolve_report;
mod report_style;
mod sim;

use clap::{Parser, Subcommand};
use tsot::card::{Card, CardRegistry, CardType, CostSource};

use cli_balance_probe::BalanceProbeArgs;
use cli_champions_report::ChampionsReportArgs;
use cli_curate::CurateBaselinesArgs;
use cli_curve_sample::CurveSampleArgs;
use cli_evolve::EvolveArgs;
use cli_matchup_evolved::MatchupEvolvedArgs;
use cli_prune_champions::PruneChampionsArgs;

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
    /// Cluster champions by Jaccard, live-rank within each cluster,
    /// keep top K per cluster, delete the rest. Bounds gauntlet by
    /// (archetypes × K), not by round count.
    PruneChampions(PruneChampionsArgs),
    /// Side-by-side comparison of card variants: each variant is
    /// pinned into every genome of a short EA; the resulting ceiling
    /// fitness measures how strong the variant is when forced in.
    BalanceProbe(BalanceProbeArgs),
    /// Play N random-deck vs random-deck games and dump a per-card
    /// turn-of-play distribution to `card-curve.json`. Consumed by
    /// `cards-report.lua` to add a turn-curve column to the pool
    /// dashboard.
    CurveSample(CurveSampleArgs),
}

/// Parse a u64 from `--seed`, accepting hex (`0xEA03`) or decimal.
/// Used by every subcommand that takes a seed flag.
pub(crate) fn parse_u64_hex_or_dec(s: &str) -> Result<u64, std::num::ParseIntError> {
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(rest, 16)
    } else {
        s.parse::<u64>()
    }
}

fn main() -> mlua::Result<()> {
    // External-observability heartbeat. Independent thread; prints
    // [ALIVE] every 2s regardless of what the main thread is doing.
    // If main hangs inside a Lua handler, deep Rust call, or anywhere
    // else, [ALIVE] keeps printing → distinguishes "alive but stuck"
    // from "killed by OS" (both stop output simultaneously). Reads the
    // last-known main-thread checkpoint so the line tells you where
    // main was when it last cooperated.
    std::thread::spawn(|| {
        let start = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let last = tsot::game::read_checkpoint();
            eprintln!(
                "[ALIVE] elapsed={:.0}s last={}",
                start.elapsed().as_secs_f64(),
                if last.is_empty() { "(none)" } else { &last },
            );
        }
    });
    // Parse args FIRST so `--help` / `--version` short-circuit before the
    // 70+ Lua cards load. Otherwise `tsot evolve --help` takes a second
    // just to print help text.
    let cli = Cli::parse();
    let registry = CardRegistry::load_embedded()?;
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
        // Balance-probe variants are excluded from the main pool — they
        // only exist for `tsot balance-probe` and shouldn't pollute
        // `evolve` / `champions-report` / gauntlet curation.
        .filter(|c| !c.is_variant)
        .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
        .filter(|c| {
            c.cost.iter().all(|cc| {
                matches!(
                    cc.source,
                    CostSource::Hand
                        | CostSource::Mill
                        | CostSource::Graveyard
                        | CostSource::Sacrifice
                        | CostSource::Attached
                )
            })
        })
        // X-cost spells without an `on_play` handler are no-ops if
        // cast — the cost is paid but nothing happens. Filter them
        // out so the EA doesn't waste budget exploring traps. Hydra
        // (creature) is unaffected because its effect lives in a
        // passive static, not on_play. Recast / turn-back-time are
        // already filtered by the SelfExile rule above.
        .filter(|c| {
            let has_x = c.cost.iter().any(|cc| cc.is_x);
            let is_spell = matches!(c.kind, CardType::Spell);
            let has_play_handler = c
                .handlers
                .keys()
                .any(|e| matches!(e, tsot::card::EventName::OnPlay));
            !(has_x && is_spell && !has_play_handler)
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

    match cli.command {
        Some(Command::Evolve(args)) => cli_evolve::run_ea(&registry, &playable_pool, &args),
        Some(Command::ChampionsReport(args)) => {
            cli_champions_report::run_champions_report(&registry, &playable_pool, &args)
        }
        Some(Command::MatchupEvolved(args)) => {
            cli_matchup_evolved::run_matchup_evolved(&registry, &args)
        }
        Some(Command::CurateBaselines(args)) => {
            cli_curate::run_curate_baselines(&registry, &args)
        }
        Some(Command::PruneChampions(args)) => {
            cli_prune_champions::run_prune_champions(&registry, &args)
        }
        Some(Command::BalanceProbe(args)) => {
            cli_balance_probe::run_balance_probe(&registry, &playable_pool, &args)
        }
        Some(Command::CurveSample(args)) => {
            cli_curve_sample::run_curve_sample(&registry, &playable_pool, &args)
        }
        None => {
            eprintln!("no subcommand specified. run with --help to see the available commands.");
            std::process::exit(2);
        }
    }
}
