mod report;
mod sim;

use rand::rngs::StdRng;
use rand::SeedableRng;
use std::collections::BTreeSet;
use std::path::Path;
use tsot::card::{Card, CardRegistry, CardType, CostSource};
use tsot::game::GameState;

use sim::{
    build_random_deck, mandatory_for_variant, print_aggregate, run_game, variant_label,
    variant_pool, DeckToken, DeckVariant, GameStats, Side, VARIANTS,
};

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
fn print_deck_listing(label: &str, deck: &[String]) {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for id in deck {
        *counts.entry(id.as_str()).or_insert(0) += 1;
    }
    println!(
        "=== Last game: {label} deck ({} cards, {} unique) ===",
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

fn main() -> mlua::Result<()> {
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
    print_deck_listing("A", &last_deck_a_ids);
    print_deck_listing("B", &last_deck_b_ids);
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
