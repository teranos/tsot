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
use tsot::sim;
use tsot::sim::evolved_deck::EvolvedDeck;

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
    /// Promote up to K champions that didn't match any existing baseline
    /// cluster to new baselines. Unmatched champions are first inner-
    /// clustered among themselves at the same `--threshold` (single-
    /// linkage Jaccard), one representative per inner-cluster is picked
    /// by highest live re-eval score, and the top K representatives are
    /// written as `baseline-promoted-{stem}.json` with a `_promoted`
    /// label suffix. `0` (default) = no promotion (manual review only).
    #[arg(long = "promote-unmatched", default_value_t = 0)]
    pub promote_unmatched: usize,
    /// AI used to play the live re-evaluation games (both seats).
    /// `uct` (default) gives high-signal play so promote/upgrade
    /// decisions reflect real card-driven outcomes. `heuristic` is the
    /// legacy fast option — its play is low-signal, so promotions made
    /// under it carry noise. Both sides use the same AI to keep the
    /// comparison fair.
    #[arg(long = "opponent-ai", default_value = "uct")]
    pub opponent_ai: String,
    /// UCT iterations per pick when `--opponent-ai uct`. 10 ≈ daily
    /// budget; raise for tighter signal at proportionally higher cost.
    #[arg(long = "opponent-uct-iterations", default_value_t = 10)]
    pub opponent_uct_iterations: u32,
    /// UCT exploration constant when `--opponent-ai uct`. `sqrt(2)` is
    /// classical.
    #[arg(long = "opponent-uct-c", default_value_t = std::f64::consts::SQRT_2)]
    pub opponent_uct_c: f64,
}

use tsot::sim::diversity::jaccard;

pub fn run_curate_baselines(
    registry: &std::sync::Arc<CardRegistry>,
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

    let ai_kind = match args.opponent_ai.to_ascii_lowercase().as_str() {
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
    let ais = [ai_kind.clone(), ai_kind.clone()];
    println!("Live AI: {:?} (both seats)", ai_kind);

    let evaluate = |cand_cards: &[Card], rng: &mut StdRng| -> f64 {
        let mut wins = 0u32;
        let mut games = 0u32;
        for opp in &baseline_decks {
            for _ in 0..args.games {
                let state = GameState::new(cand_cards.to_vec(), opp.clone());
                let game_seed = rng.gen();
                let mut game_rng = StdRng::seed_from_u64(game_seed);
                let mut log: Vec<String> = Vec::new();
                let (stats, _) =
                    sim::run_game_with_ai(state, &mut game_rng, &mut log, registry, &ais, game_seed);
                if stats.winner == tsot::game::PlayerId::A {
                    wins += 1;
                }
                games += 1;
                let state = GameState::new(opp.clone(), cand_cards.to_vec());
                let game_seed = rng.gen();
                let mut game_rng = StdRng::seed_from_u64(game_seed);
                let mut log = Vec::new();
                let (stats, _) =
                    sim::run_game_with_ai(state, &mut game_rng, &mut log, registry, &ais, game_seed);
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
        for p in &unmatched {
            println!("  {}", p.display());
        }
    }

    // Promotion phase: take up to `--promote-unmatched K` representatives
    // from inner-clusters of the unmatched champions and write them as
    // new baselines. The inner-cluster step prevents promoting K slot-
    // variations of one new attractor (e.g. a single round's rank-1..5
    // when --save-top 5 saved one cluster's worth of clones).
    if args.promote_unmatched > 0 && !unmatched.is_empty() {
        let unmatched_entries: Vec<&(PathBuf, EvolvedDeck, Vec<Card>, BTreeSet<String>)> =
            champions
                .iter()
                .filter(|(p, _, _, _)| !all_matched.contains(p))
                .collect();

        println!();
        println!(
            "Promotion phase: {} unmatched champions, K={}, inner-cluster threshold {:.2}",
            unmatched_entries.len(),
            args.promote_unmatched,
            args.threshold,
        );

        // Live-score each unmatched champion against the snapshot baselines.
        // (The main upgrade loop only scored cluster members.)
        let mut scores: Vec<f64> = Vec::with_capacity(unmatched_entries.len());
        for (path, _, cards, _) in &unmatched_entries {
            let s = evaluate(cards, &mut rng);
            scores.push(s);
            println!(
                "  {:<40}  live={:.3}",
                path.file_name().unwrap().to_string_lossy(),
                s,
            );
        }

        // Single-linkage Jaccard clustering among unmatched champions.
        let n = unmatched_entries.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        for i in 0..n {
            for j in (i + 1)..n {
                let si = &unmatched_entries[i].3;
                let sj = &unmatched_entries[j].3;
                if jaccard(si, sj) >= args.threshold {
                    let ri = find(&mut parent, i);
                    let rj = find(&mut parent, j);
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }
        let mut clusters: std::collections::BTreeMap<usize, Vec<usize>> =
            std::collections::BTreeMap::new();
        for i in 0..n {
            let r = find(&mut parent, i);
            clusters.entry(r).or_default().push(i);
        }
        let cluster_list: Vec<Vec<usize>> = clusters.into_values().collect();

        // Pick the highest-live-score member per inner-cluster.
        let mut reps: Vec<(usize, f64)> = cluster_list
            .iter()
            .map(|members| {
                let best = *members
                    .iter()
                    .max_by(|a, b| {
                        scores[**a]
                            .partial_cmp(&scores[**b])
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap();
                (best, scores[best])
            })
            .collect();
        reps.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let take = reps.len().min(args.promote_unmatched);
        println!();
        println!(
            "  → {} distinct inner-cluster(s); promoting top {} by live score:",
            cluster_list.len(),
            take,
        );
        let mut promoted_count = 0u32;
        for (rank, (idx, score)) in reps.iter().take(take).enumerate() {
            let (src_path, src_deck, _, _) = unmatched_entries[*idx];
            let stem = src_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            let target = baselines_dir.join(format!("baseline-promoted-{stem}.json"));
            let mut promoted = src_deck.clone();
            promoted.fitness = *score;
            promoted.label = format!("{}_promoted", promoted.label);
            if args.dry_run {
                println!(
                    "  → rank {}: would promote {} → {} (live={:.3}) [dry-run]",
                    rank + 1,
                    src_path.file_name().unwrap().to_string_lossy(),
                    target.file_name().unwrap().to_string_lossy(),
                    score,
                );
            } else {
                match promoted.save(&target) {
                    Ok(()) => {
                        println!(
                            "  → rank {}: PROMOTED {} → {} (live={:.3})",
                            rank + 1,
                            src_path.file_name().unwrap().to_string_lossy(),
                            target.file_name().unwrap().to_string_lossy(),
                            score,
                        );
                        promoted_count += 1;
                    }
                    Err(e) => eprintln!("  ! save failed for {}: {e}", target.display()),
                }
            }
        }
        println!();
        println!(
            "Promotion done: {} new baseline(s) written",
            promoted_count,
        );
    }
    Ok(())
}
