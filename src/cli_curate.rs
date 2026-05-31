//! `tsot curate-baselines` subcommand: for each baseline, find champions
//! in its Jaccard cluster, evaluate every cluster member live against
//! the snapshot baselines, replace the baseline with the highest live-
//! win-rate candidate. Apples-to-apples comparison — no saved-fitness
//! bias from cross-round gauntlets.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::Parser;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::{Card, CardRegistry};
use tsot::game::GameState;

use crate::parse_u64_hex_or_dec;
use crate::sim;
use crate::sim::evolved_deck::EvolvedDeck;

#[derive(Parser)]
pub struct CurateBaselinesArgs {
    /// Directory of champion candidates to consider for promotion.
    #[arg(long, default_value = "champions")]
    pub dir: String,
    /// Directory of baselines to upgrade in place.
    #[arg(long, default_value = "baselines")]
    pub baselines: String,
    /// Jaccard threshold for cluster membership (candidate is in
    /// baseline B's cluster if Jaccard(candidate, B) >= this).
    #[arg(long, default_value_t = 0.7)]
    pub threshold: f64,
    /// Games per side per (candidate, baseline) pairing during live
    /// re-evaluation. Total games per candidate = 2 × baselines × games.
    #[arg(long, default_value_t = 20)]
    pub games: u32,
    /// Master seed for the live evaluation RNG.
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
    pub seed: u64,
    /// Don't overwrite baseline files; print what would happen.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

pub fn run_curate_baselines(
    registry: &CardRegistry,
    args: &CurateBaselinesArgs,
) -> mlua::Result<()> {
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

    let baseline_decks: Vec<Vec<Card>> = baselines.iter().map(|(_, _, c, _)| c.clone()).collect();
    let mut rng = StdRng::seed_from_u64(args.seed);

    let evaluate = |cand_cards: &[Card], rng: &mut StdRng| -> f64 {
        let mut wins = 0u32;
        let mut games = 0u32;
        for opp in &baseline_decks {
            for _ in 0..args.games {
                let state = GameState::new(cand_cards.to_vec(), opp.clone());
                let mut game_rng = StdRng::seed_from_u64(rng.gen());
                let mut log: Vec<String> = Vec::new();
                let (stats, _) = sim::run_game(state, &mut game_rng, &mut log, registry.lua());
                if stats.winner == tsot::game::PlayerId::A {
                    wins += 1;
                }
                games += 1;
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
    Ok(())
}
