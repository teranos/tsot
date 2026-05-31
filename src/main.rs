mod champions_report;
mod cli_champions_report;
mod cli_curate;
mod cli_evolve;
mod cli_matchup_evolved;
mod cli_prune_champions;
mod evolve_report;
mod report_style;
mod sim;

use clap::{Parser, Subcommand};
use std::path::Path;
use tsot::card::{Card, CardRegistry, CardType, CostSource};

use cli_champions_report::ChampionsReportArgs;
use cli_curate::CurateBaselinesArgs;
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
    // Parse args FIRST so `--help` / `--version` short-circuit before the
    // 70+ Lua cards load. Otherwise `tsot evolve --help` takes a second
    // just to print help text.
    let cli = Cli::parse();
    let registry = CardRegistry::load(Path::new("cards"))?;
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
        None => {
            eprintln!("no subcommand specified. run with --help to see the available commands.");
            std::process::exit(2);
        }
    }
}
