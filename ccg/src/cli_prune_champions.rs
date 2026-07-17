//! `tsot prune-champions` subcommand: bound the champion pool by
//! archetype × keep-K. Two-step process:
//!
//!   1. Cluster every champion in `--dir` by Jaccard similarity on
//!      card-id sets (same single-linkage clustering the archetype
//!      dashboard uses).
//!   2. Within each cluster, live-evaluate every member against the
//!      snapshot baselines (apples-to-apples — no saved-fitness bias
//!      from cross-round gauntlets). Keep the top `--keep-per-cluster`
//!      by live score. Delete the rest.
//!
//! Use case: as `make evolve` rounds accumulate, the gauntlet of
//! `champions/*.json` grows linearly. Most rounds produce 5 ranks that
//! are slot-variations of one archetype, so 80% of the pool is
//! redundant for gauntlet diversity. This subcommand keeps the
//! diversity, drops the duplicates.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::Parser;
use maud::{html, Markup, PreEscaped, DOCTYPE};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::{Card, CardRegistry};
use tsot::game::GameState;

use crate::parse_u64_hex_or_dec;
use crate::report_style;
use tsot::sim;
use tsot::sim::evolved_deck::EvolvedDeck;

#[derive(Parser)]
pub struct PruneChampionsArgs {
    /// Directory of champions to prune in place.
    #[arg(long, default_value = "champions")]
    pub dir: String,
    /// Directory of baselines used as the live-eval opponent pool.
    #[arg(long, default_value = "baselines")]
    pub baselines: String,
    /// Jaccard threshold for cluster membership. Champions with
    /// Jaccard ≥ this share a cluster (single-linkage).
    #[arg(long, default_value_t = 0.4)]
    pub threshold: f64,
    /// How many champions to keep per cluster (sorted by live score
    /// descending). Smaller = tighter gauntlet, less compute. Default
    /// 2 keeps the best + a backup per archetype.
    #[arg(long = "keep-per-cluster", default_value_t = 2)]
    pub keep: usize,
    /// Games per side per (champion, baseline) pairing during live
    /// re-evaluation. Total games per champion = 2 × baselines × games.
    #[arg(long, default_value_t = 20)]
    pub games: u32,
    /// Master seed for the live-evaluation RNG.
    #[arg(long, default_value_t = 0xEA_C8, value_parser = parse_u64_hex_or_dec)]
    pub seed: u64,
    /// Don't delete files; print what would happen.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Path to write the HTML prune report. Pass `-` to skip.
    #[arg(long = "html-report", default_value = "prune-report.html")]
    pub html_report: String,
}

struct ReportRow {
    name: String,
    live_score: f64,
    rank_in_cluster: usize,
    kept: bool,
}

struct ReportCluster {
    id: usize,
    size: usize,
    rows: Vec<ReportRow>,
    /// (card_id, in_count) — cards present in ≥ half the cluster members.
    signature: Vec<(String, usize)>,
}

fn write_html_report(
    path: &str,
    args: &PruneChampionsArgs,
    pool: &[Card],
    clusters: &[ReportCluster],
    kept_count: usize,
    delete_count: usize,
    applied: bool,
) -> std::io::Result<()> {
    let markup = build_html(args, pool, clusters, kept_count, delete_count, applied);
    std::fs::write(path, markup.into_string())
}

fn build_html(
    args: &PruneChampionsArgs,
    pool: &[Card],
    clusters: &[ReportCluster],
    kept_count: usize,
    delete_count: usize,
    applied: bool,
) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "tsot — prune champions" }
                style { (PreEscaped(report_style::CSS)) }
                style { "
                    .cluster-card { border: 1px solid var(--border); border-left: 3px solid var(--accent);
                                    padding: 8px 12px; margin: 0 0 1em; background: var(--bg-panel); }
                    .cluster-card.singleton { border-left-color: var(--text-tertiary); }
                    .cluster-card h3 { margin: 0 0 0.4em; color: var(--text-emphasis); font-size: 14px; }
                    .cluster-card .sigcards { color: var(--text-secondary); font-size: 11px;
                                              margin-bottom: 6px; }
                    .cluster-card .sigcards b { color: var(--text); }
                    tr.deleted td { color: var(--text-tertiary); }
                    tr.deleted td.decision { color: #c87060; }
                    tr.kept td.decision { color: var(--accent); font-weight: 600; }
                    .applied-tag { color: #c87060; font-weight: 600; }
                    .preview-tag { color: var(--accent); font-weight: 600; }
                " }
            }
            body {
                h1 { "tsot — prune champions" }
                div.meta {
                    div { span.k { "champions before" } b { (kept_count + delete_count) } }
                    div { span.k { "kept" } b { (kept_count) } }
                    div { span.k { "deleted" } b { (delete_count) } }
                    div { span.k { "clusters" } b { (clusters.len()) } }
                    div { span.k { "threshold" } b { (format!("{:.2}", args.threshold)) } }
                    div { span.k { "keep/cluster" } b { (args.keep) } }
                    div { span.k { "games/pairing" } b { (args.games) } }
                    div {
                        @if applied {
                            span.applied-tag { "DELETED" }
                        } @else {
                            span.preview-tag { "DRY-RUN PREVIEW" }
                        }
                    }
                }

                p.note {
                    "Champions are clustered single-linkage by Jaccard ≥ "
                    (format!("{:.2}", args.threshold))
                    " on their card-id sets. Within each cluster, every member is "
                    "live-re-evaluated against the snapshot baselines ("
                    (args.games) " games per side × baselines). The top "
                    (args.keep) " per cluster by live score are kept; the rest are "
                    @if applied { "deleted." } @else { "marked for deletion (preview only — re-run without --dry-run to apply)." }
                }

                @for cluster in clusters {
                    @let is_singleton = cluster.size == 1;
                    div class=(if is_singleton { "cluster-card singleton" } else { "cluster-card" }) {
                        h3 { "Cluster " (cluster.id) " · " (cluster.size) " member"
                             (if cluster.size == 1 { "" } else { "s" }) }
                        @if !cluster.signature.is_empty() {
                            div.sigcards {
                                b { "signature cards (≥ ½ of cluster):" }
                                div.mini-card-row {
                                    @for (cid, in_count) in &cluster.signature {
                                        (report_style::mini_card(pool, cid, *in_count, cluster.size))
                                    }
                                }
                            }
                        }
                        table.summary {
                            thead {
                                tr {
                                    th { "rank" }
                                    th { "champion" }
                                    th.num { "live score" }
                                    th { "decision" }
                                }
                            }
                            tbody {
                                @for row in &cluster.rows {
                                    tr class=(if row.kept { "kept" } else { "deleted" }) {
                                        td.num { (row.rank_in_cluster) }
                                        td { (row.name) }
                                        td.num { (format!("{:.3}", row.live_score)) }
                                        td.decision {
                                            @if row.kept { "KEEP" } @else {
                                                @if applied { "DELETED" } @else { "would delete" }
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
}

use tsot::sim::diversity::jaccard;

pub fn run_prune_champions(
    registry: &std::sync::Arc<CardRegistry>,
    args: &PruneChampionsArgs,
) -> mlua::Result<()> {
    let baselines_dir = std::path::Path::new(&args.baselines);
    let champions_dir = std::path::Path::new(&args.dir);

    // Load baselines (the live-eval opponent pool).
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

    let baseline_decks: Vec<Vec<tsot::game::DeckUnit>> = baseline_paths
        .iter()
        .filter_map(|p| EvolvedDeck::load(p).ok()?.to_units(registry).ok())
        .collect();

    // Load champions.
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

    if champion_paths.is_empty() {
        println!("no champions to prune in {}", champions_dir.display());
        return Ok(());
    }

    // Parse each champion: keep path + materialized cards + id set.
    struct ChampEntry {
        path: PathBuf,
        units: Vec<tsot::game::DeckUnit>,
        ids: BTreeSet<String>,
    }
    let mut champs: Vec<ChampEntry> = Vec::new();
    for p in &champion_paths {
        let Ok(deck) = EvolvedDeck::load(p) else {
            eprintln!("warn: skipping unloadable {}", p.display());
            continue;
        };
        let Ok(units) = deck.to_units(registry) else {
            eprintln!("warn: skipping (units: missing card id) {}", p.display());
            continue;
        };
        let ids: BTreeSet<String> = deck.card_ids.iter().cloned().collect();
        champs.push(ChampEntry {
            path: p.clone(),
            units,
            ids,
        });
    }

    println!(
        "Prune-champions: {} champions × {} baselines, threshold {:.2}, keep {} per cluster, {} games/side",
        champs.len(),
        baseline_decks.len(),
        args.threshold,
        args.keep,
        args.games,
    );

    // Single-linkage Jaccard clustering via union-find on indices.
    let n = champs.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while parent[r] != r {
            r = parent[r];
        }
        let mut x = i;
        while parent[x] != r {
            let next = parent[x];
            parent[x] = r;
            x = next;
        }
        r
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if jaccard(&champs[i].ids, &champs[j].ids) >= args.threshold {
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
    let mut cluster_list: Vec<Vec<usize>> = clusters.into_values().collect();
    cluster_list.sort_by_key(|c| std::cmp::Reverse(c.len()));

    // Live-eval each champion against the baselines.
    let mut rng = StdRng::seed_from_u64(args.seed);
    let evaluate = |units: &[tsot::game::DeckUnit], rng: &mut StdRng| -> f64 {
        let mut wins = 0u32;
        let mut games = 0u32;
        for opp in &baseline_decks {
            for _ in 0..args.games {
                // candidate as A
                let state = GameState::from_units(units.to_vec(), opp.clone());
                let game_seed = rng.gen();
                let mut game_rng = StdRng::seed_from_u64(game_seed);
                let mut log: Vec<String> = Vec::new();
                let (stats, _) = sim::run_game(state, &mut game_rng, &mut log, registry, game_seed);
                if stats.winner == tsot::game::PlayerId::A {
                    wins += 1;
                }
                games += 1;
                // candidate as B
                let state = GameState::from_units(opp.clone(), units.to_vec());
                let game_seed = rng.gen();
                let mut game_rng = StdRng::seed_from_u64(game_seed);
                let mut log = Vec::new();
                let (stats, _) = sim::run_game(state, &mut game_rng, &mut log, registry, game_seed);
                if stats.winner == tsot::game::PlayerId::B {
                    wins += 1;
                }
                games += 1;
            }
        }
        wins as f64 / games as f64
    };

    let mut keep: BTreeSet<PathBuf> = BTreeSet::new();
    let mut delete: Vec<(PathBuf, f64)> = Vec::new();
    let mut report_clusters: Vec<ReportCluster> = Vec::new();

    for (cidx, indices) in cluster_list.iter().enumerate() {
        let size = indices.len();
        println!();
        println!("Cluster {} ({} member{}):", cidx + 1, size, if size == 1 { "" } else { "s" });
        let mut scored: Vec<(usize, f64)> = Vec::with_capacity(size);
        for &i in indices {
            let live = evaluate(&champs[i].units, &mut rng);
            scored.push((i, live));
            println!(
                "  {:<35}  live={:.3}",
                champs[i].path.file_name().unwrap().to_string_lossy(),
                live
            );
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let cutoff = args.keep.min(scored.len());

        // Signature cards: ids appearing in ≥ ceil(size/2) cluster members.
        let mut card_count: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for &i in indices {
            for cid in &champs[i].ids {
                *card_count.entry(cid.clone()).or_insert(0) += 1;
            }
        }
        let half = size.div_ceil(2);
        let mut signature: Vec<(String, usize)> = card_count
            .into_iter()
            .filter(|(_, c)| *c >= half)
            .collect();
        signature.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

        let mut report_rows: Vec<ReportRow> = Vec::with_capacity(scored.len());
        for (rank, (i, live)) in scored.iter().enumerate() {
            let name = champs[*i]
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let kept = rank < cutoff;
            if kept {
                println!("  → KEEP {}  (rank {} in cluster, live={:.3})", name, rank + 1, live);
                keep.insert(champs[*i].path.clone());
            } else {
                println!("  → DELETE {}  (rank {} in cluster, live={:.3})", name, rank + 1, live);
                delete.push((champs[*i].path.clone(), *live));
            }
            report_rows.push(ReportRow {
                name,
                live_score: *live,
                rank_in_cluster: rank + 1,
                kept,
            });
        }

        report_clusters.push(ReportCluster {
            id: cidx + 1,
            size,
            rows: report_rows,
            signature,
        });
    }

    println!();
    println!(
        "Summary: {} clusters · {} champions kept · {} would-delete",
        cluster_list.len(),
        keep.len(),
        delete.len(),
    );

    let kept_count = keep.len();
    let delete_count = delete.len();
    let mut applied = false;

    if delete.is_empty() {
        println!("Nothing to delete.");
    } else if args.dry_run {
        println!("[dry-run] no files removed. Re-run without --dry-run to apply.");
    } else {
        let mut removed = 0usize;
        for (p, _) in &delete {
            match std::fs::remove_file(p) {
                Ok(()) => removed += 1,
                Err(e) => eprintln!("  ! failed to delete {}: {e}", p.display()),
            }
        }
        println!("Deleted {removed} file(s).");
        applied = true;
    }

    // Write the HTML prune report unless explicitly skipped.
    if args.html_report != "-" {
        match write_html_report(
            &args.html_report,
            args,
            registry.cards(),
            &report_clusters,
            kept_count,
            delete_count,
            applied,
        ) {
            Ok(()) => println!("Prune report written to {}", args.html_report),
            Err(e) => eprintln!("failed to write prune report: {e}"),
        }
    }

    Ok(())
}
