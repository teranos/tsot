mod report;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tsot::card::{Card, CardRegistry, CardType, CostSource, EventName};
use tsot::choice::{
    ChoiceOracle, ChooseIntRequest, RandomOracle, RecordingOracle, ScriptedOracle,
};
use tsot::game::{EventContext, GameState, InstanceId, Phase, PlayChoices, PlayerId};

/// Master seed for the sim's RNG. Default: fresh per run (sampled from
/// system entropy via `StdRng::from_entropy`) so normal `cargo run`
/// shows varied games. Override via env var `TSOT_SEED=<integer>` for
/// reproducible runs (replay, regression debugging, before/after card
/// comparisons).
fn pick_seed() -> u64 {
    std::env::var("TSOT_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| {
            use rand::RngCore;
            StdRng::from_entropy().next_u64()
        })
}

/// Games per matchup cell (25 cells × this = total iterations). Default
/// chosen for ~tight win-rate intervals (±5% at 95% confidence). Override
/// with `TSOT_GAMES_PER_MATCHUP=<n>` env var.
const DEFAULT_GAMES_PER_MATCHUP: usize = 100;

fn games_per_matchup() -> usize {
    std::env::var("TSOT_GAMES_PER_MATCHUP")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_GAMES_PER_MATCHUP)
}

/// Deck-build variants. A and B are full-pool baselines; H/G/U are
/// filtered pools meant to stress-test specific corpus interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DeckVariant {
    /// Full pool, no filter.
    A,
    /// Full pool, no filter (identical to A; kept distinct so the matchup
    /// matrix shows the A↔A and A↔B baselines symmetrically).
    B,
    /// No goblins — exercises anti-tribal play against goblin-warlord etc.
    H,
    /// No humans, fish, insects, or beasts.
    G,
    /// Colorless or blue only — heavy on draw / counter / interaction.
    U,
    /// Red or purple cards (must list at least one of those colors).
    R,
}

pub(crate) const VARIANTS: [DeckVariant; 6] = [
    DeckVariant::A,
    DeckVariant::B,
    DeckVariant::H,
    DeckVariant::G,
    DeckVariant::U,
    DeckVariant::R,
];

pub(crate) fn variant_label(v: DeckVariant) -> &'static str {
    match v {
        DeckVariant::A => "A",
        DeckVariant::B => "B",
        DeckVariant::H => "H",
        DeckVariant::G => "G",
        DeckVariant::U => "U",
        DeckVariant::R => "R",
    }
}

fn variant_pool(playable: &[Card], v: DeckVariant) -> Vec<Card> {
    match v {
        DeckVariant::A | DeckVariant::B => playable.to_vec(),
        DeckVariant::H => playable
            .iter()
            .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("goblin")))
            .cloned()
            .collect(),
        DeckVariant::G => playable
            .iter()
            .filter(|c| {
                !c.subtypes.iter().any(|s| {
                    s.eq_ignore_ascii_case("human")
                        || s.eq_ignore_ascii_case("fish")
                        || s.eq_ignore_ascii_case("insect")
                        || s.eq_ignore_ascii_case("beast")
                })
            })
            .cloned()
            .collect(),
        DeckVariant::U => playable
            .iter()
            .filter(|c| {
                c.colors.is_empty()
                    || c.colors.iter().any(|col| col.eq_ignore_ascii_case("blue"))
            })
            .cloned()
            .collect(),
        DeckVariant::R => playable
            .iter()
            .filter(|c| {
                c.colors.iter().any(|col| {
                    col.eq_ignore_ascii_case("red") || col.eq_ignore_ascii_case("purple")
                })
            })
            .cloned()
            .collect(),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GameStats {
    turns: u32,
    winner: PlayerId,
    variant_a: DeckVariant,
    variant_b: DeckVariant,
    /// Unique card IDs in A's starting deck. Same card repeated in the
    /// 50-card deck only counts once. Used for per-card win-rate analysis
    /// in the HTML report.
    deck_a_ids: BTreeSet<String>,
    deck_b_ids: BTreeSet<String>,
    /// Unique card IDs that actually got played at least once during the
    /// game (via the play loop). Compared against `deck_*_ids` to surface
    /// "was this card drawn-and-played" vs "just sitting in the deck."
    a_played_card_ids: BTreeSet<String>,
    b_played_card_ids: BTreeSet<String>,
    a_played: u32,
    b_played: u32,
    a_attacks: u32,
    b_attacks: u32,
    a_deaths: u32,
    b_deaths: u32,
    a_milled_to_exile: u32, // cards exiled from A's deck by B's combat damage
    b_milled_to_exile: u32, // cards exiled from B's deck by A's combat damage
    a_final_board: u32,
    b_final_board: u32,
    a_final_gy: u32,
    b_final_gy: u32,
    // Future-simulation telemetry — every play opens a journal.
    a_preview_attempts: u32,
    b_preview_attempts: u32,
    a_preview_rollbacks: u32,
    b_preview_rollbacks: u32,
    a_preview_journal_size_total: u64,
    b_preview_journal_size_total: u64,
    // Game-long replay journal: total mutations captured across all
    // committed plays + engine-driven mutations (turn flow, combat, etc.).
    replay_journal_entries: u64,
    event_fires: BTreeMap<EventName, [u32; 2]>,
    action_counts: BTreeMap<String, [u32; 2]>,
}

fn main() -> mlua::Result<()> {
    let registry = CardRegistry::load(Path::new("cards"))?;
    // Pool for deck construction: creatures + instants whose costs the engine
    // currently supports (HAND, MILL, GRAVEYARD; no variable X; no SACRIFICE/SELF).
    // Test subtype excluded per S.5.
    let playable_pool: Vec<Card> = registry
        .cards()
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                CardType::Creature | CardType::Spell | CardType::Artifact
            )
        })
        .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
        .filter(|c| {
            c.cost.iter().all(|cc| {
                matches!(
                    cc.source,
                    CostSource::Hand | CostSource::Mill | CostSource::Graveyard
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
    // Pre-build the per-variant pools once — pure subsets of playable_pool.
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

    // Deterministic matchup cycling: each (A_variant, B_variant) cell gets
    // exactly `games_per_cell` games. Total = 25 × games_per_cell.
    for &v_a in &VARIANTS {
        for &v_b in &VARIANTS {
            let pool_a = &pools.iter().find(|(v, _)| *v == v_a).unwrap().1;
            let pool_b = &pools.iter().find(|(v, _)| *v == v_b).unwrap().1;
            for _ in 0..games_per_cell {
                let deck_a = build_random_deck(pool_a, &mut rng, 50);
                let deck_b = build_random_deck(pool_b, &mut rng, 50);
                last_deck_a_ids = deck_a.iter().map(|c| c.id.clone()).collect();
                last_deck_b_ids = deck_b.iter().map(|c| c.id.clone()).collect();
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
                all.push(stats);
                last_journal = journal;
            }
        }
    }
    let elapsed = t0.elapsed();

    // If TSOT_REPLAY_OUT is set, dump the last game's ReplayFile to JSON.
    if let Some(path) = replay_out_path.as_ref() {
        let replay = tsot::replay::ReplayFile {
            seed,
            deck_a_card_ids: last_deck_a_ids,
            deck_b_card_ids: last_deck_b_ids,
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

    // Write the HTML report. Default path: tsot-report.html in cwd.
    // Override with TSOT_REPORT_OUT=<path>. Set TSOT_REPORT_OUT=- to skip.
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

fn build_random_deck(pool: &[Card], rng: &mut impl Rng, size: usize) -> Vec<Card> {
    let mut deck: Vec<Card> = (0..size)
        .map(|_| pool.choose(rng).unwrap().clone())
        .collect();
    deck.shuffle(rng);
    deck
}

fn run_game(
    mut state: GameState,
    rng: &mut StdRng,
    log: &mut Vec<String>,
    lua: &mlua::Lua,
) -> (GameStats, tsot::game::Journal) {
    // Oracle RNG derived from the master RNG so the whole sim is reproducible
    // from one seed.
    let oracle_seed: u64 = rng.gen();
    let mut oracle = RecordingOracle::new(RandomOracle::new(StdRng::seed_from_u64(oracle_seed)));

    // Open a game-long replay journal. Every committed mutation will be
    // recorded into this for the duration of the game; previewed-and-skipped
    // mutations stay isolated in the per-action `state.journal`.
    state.replay_journal = Some(tsot::game::Journal::new());
    let mut stats = GameStats {
        turns: 0,
        winner: PlayerId::A,
        // Caller overwrites these after run_game returns.
        variant_a: DeckVariant::A,
        variant_b: DeckVariant::B,
        deck_a_ids: BTreeSet::new(),
        deck_b_ids: BTreeSet::new(),
        a_played_card_ids: BTreeSet::new(),
        b_played_card_ids: BTreeSet::new(),
        a_played: 0,
        b_played: 0,
        a_attacks: 0,
        b_attacks: 0,
        a_deaths: 0,
        b_deaths: 0,
        a_milled_to_exile: 0,
        b_milled_to_exile: 0,
        a_final_board: 0,
        b_final_board: 0,
        a_final_gy: 0,
        b_final_gy: 0,
        a_preview_attempts: 0,
        b_preview_attempts: 0,
        a_preview_rollbacks: 0,
        b_preview_rollbacks: 0,
        a_preview_journal_size_total: 0,
        b_preview_journal_size_total: 0,
        replay_journal_entries: 0,
        event_fires: BTreeMap::new(),
        action_counts: BTreeMap::new(),
    };

    let mut safety = 1000;
    while state.winner.is_none() && safety > 0 {
        safety -= 1;
        let active = state.active_player;
        let turn = state.turn;
        let mut events: Vec<String> = Vec::new();

        // Advance to Main1.
        while state.phase != Phase::Main1 && state.winner.is_none() {
            state.next_phase();
        }
        if state.winner.is_some() {
            log.push(format!("turn {turn} ({active:?}): deck-out before Main1"));
            break;
        }

        // TODO(stack-phase-2-sim): once instants + the response window exist,
        // this loop needs a second decision point: between an opponent's
        // stack-item being added and resolving, the sim AI must decide
        // whether to play an instant in response (or pass). Today the sim
        // only acts on its own Main1 and never sees a response window.
        //
        // Multi-card-per-turn (Pattern A): the AI plays at most one
        // creature AND at most one non-creature per turn. After the first
        // play, the kind filter constrains the second pick to a different
        // kind. The inner loop breaks when both slots are used or no
        // eligible card is found.
        let mut played_creature = false;
        let mut played_noncreature = false;
        while !(played_creature && played_noncreature) && state.winner.is_none() {
            let kind_filter = if played_creature {
                PickKindFilter::NonCreatureOnly
            } else if played_noncreature {
                PickKindFilter::CreatureOnly
            } else {
                PickKindFilter::Any
            };
            let Some(picked) = pick_random_playable_in_hand(&state, active, rng, kind_filter)
            else {
                break;
            };
            let picked_is_creature = state
                .card_pool
                .get(&picked)
                .map(|c| c.card.kind == CardType::Creature)
                .unwrap_or(false);
            // Indent: the existing block needs to know `picked` and produce
            // play outcome that we feed back into the loop flags.
            {
            let kind = state
                .card_pool
                .get(&picked)
                .map(|c| c.card.kind)
                .unwrap_or(CardType::Unspecified);
            let mut choices = PlayChoices::default();
            // Variable-X handling: if the card has an is_x cost component,
            // ask the oracle for X and build the hand payment accordingly.
            // No rigging — X-cost cards earn their attached count by paying it.
            let cost = state
                .card_pool
                .get(&picked)
                .map(|c| c.card.cost.clone())
                .unwrap_or_default();
            let has_is_x = cost.iter().any(|c| c.is_x);

            if has_is_x {
                let hand_size = state.player(active).hand.len();
                // Exclude the played card from the upper bound; cap for sanity.
                let max_x = (hand_size.saturating_sub(1)).min(10) as i32;
                let x = oracle.choose_int(
                    &state,
                    ChooseIntRequest {
                        min: 0,
                        max: max_x,
                        prompt: format!("X for {}", short(&picked)),
                    },
                );
                state.bump_action("choose_int", active);
                choices.x_value = Some(x);
                let hand_needed: usize = cost
                    .iter()
                    .filter(|c| c.is_x && matches!(c.source, CostSource::Hand))
                    .map(|_| x.max(0) as usize)
                    .sum();
                if hand_needed > 0 {
                    choices.hand_payment_ids =
                        state.resolve_hand_payment(active, &picked, hand_needed, &mut oracle);
                }
            } else if matches!(kind, CardType::Creature) {
                rig_creature_free_haste(&mut state, &picked);
            } else if matches!(kind, CardType::Spell) {
                // HAND cost: ask the oracle slot-by-slot which card to spend.
                // Recorded by RecordingOracle so retry-on-suicide sees it.
                let hand_needed: usize = cost
                    .iter()
                    .filter(|c| matches!(c.source, CostSource::Hand))
                    .map(|c| c.amount.max(0) as usize)
                    .sum();
                if hand_needed > 0 {
                    choices.hand_payment_ids =
                        state.resolve_hand_payment(active, &picked, hand_needed, &mut oracle);
                }
            }
            // Preview-and-skip: open a journal, attempt the play. If the
            // play would deck the active player (suicide), rollback and
            // skip. Otherwise discard the journal and keep the mutations.
            oracle.clear();
            // Snapshot response-played count so we can detect whether the
            // response policy fired during this preview. If it did, the
            // ScriptedOracle replay can't reproduce the same call sequence
            // (its respond_or_pass defaults to Pass) — gate retry to skip.
            let resp_before_a = state
                .action_counts
                .get("instant_response_played")
                .map(|v| v[0])
                .unwrap_or(0);
            let resp_before_b = state
                .action_counts
                .get("instant_response_played")
                .map(|v| v[1])
                .unwrap_or(0);
            state.journal = Some(tsot::game::Journal::new());
            let opponent_of_active = active.opponent();
            let choices_for_retry = choices.clone();
            let result = state.play_card(
                active,
                &picked,
                choices,
                Some(&mut EventContext::new(lua, &mut oracle)),
            );
            let resp_after_a = state
                .action_counts
                .get("instant_response_played")
                .map(|v| v[0])
                .unwrap_or(0);
            let resp_after_b = state
                .action_counts
                .get("instant_response_played")
                .map(|v| v[1])
                .unwrap_or(0);
            let response_fired =
                resp_after_a > resp_before_a || resp_after_b > resp_before_b;
            let mut suicide = state.winner == Some(opponent_of_active);
            let preview_size = state.journal.as_ref().map(|j| j.len()).unwrap_or(0) as u64;

            // Telemetry: every play opened a journal, so we count it.
            bump_preview_attempt(&mut stats, active, preview_size);

            // Retry-on-suicide: if the play suicided and the recording
            // contains a choose_player answer, the active player's "target
            // player" pick was the cause (or at least correlated). Roll
            // back, replay with a ScriptedOracle that flips the first
            // choose_player answer. If the flipped run survives, commit it.
            //
            // TODO(retry-eval): this is naive — any non-suicidal flipped
            // outcome is accepted over skipping. That's actively wrong when
            // we're ahead on board and the flipped play just hands the
            // opponent free cards (e.g., Field Notes → opponent draws 2-3).
            // Correct behavior needs a board-eval comparing score(skip) vs
            // score(retry); commit retry only if it scores higher. Holding
            // off until heuristic weights are designed.
            let mut result = result;
            if suicide && !response_fired {
                if let Some(flipped) = ScriptedOracle::flip_first_player(oracle.recording()) {
                    if let Some(journal) = state.journal.take() {
                        journal.rollback(&mut state);
                    }
                    state.journal = Some(tsot::game::Journal::new());
                    let mut scripted = ScriptedOracle::new(flipped);
                    result = state.play_card(
                        active,
                        &picked,
                        choices_for_retry,
                        Some(&mut EventContext::new(lua, &mut scripted)),
                    );
                    suicide = state.winner == Some(opponent_of_active);
                    if !suicide && result.is_ok() {
                        state.bump_action("preview_retry_rescued", active);
                    }
                }
            }

            if result.is_ok() && !suicide {
                // Commit: transfer preview entries into the replay journal,
                // then drop the preview.
                if let Some(mut preview) = state.journal.take() {
                    if let Some(replay) = state.replay_journal.as_mut() {
                        replay.extend_from(&mut preview);
                    }
                }
                bump_played(&mut stats, active);
                // Record card-id-was-played for per-card performance.
                if let Some(card_id) = state.card_pool.get(&picked).map(|c| c.card.id.clone()) {
                    match active {
                        PlayerId::A => {
                            stats.a_played_card_ids.insert(card_id);
                        }
                        PlayerId::B => {
                            stats.b_played_card_ids.insert(card_id);
                        }
                    }
                }
                let timing = state
                    .card_pool
                    .get(&picked)
                    .and_then(|c| c.card.timing);
                let label = match kind {
                    CardType::Spell => match timing {
                        Some(tsot::Timing::Instant) => format!("instant {}", short(&picked)),
                        Some(tsot::Timing::Sorcery) => format!("sorcery {}", short(&picked)),
                        None => format!("spell {}", short(&picked)),
                    },
                    _ => {
                        let (x, y) = state.effective_stats(&picked);
                        format!("{} ({x}/{y})", short(&picked))
                    }
                };
                events.push(format!("played {label}"));
                if picked_is_creature {
                    played_creature = true;
                } else {
                    played_noncreature = true;
                }
            } else {
                if let Some(journal) = state.journal.take() {
                    journal.rollback(&mut state);
                }
                bump_preview_rollback(&mut stats, active);
                if suicide {
                    state.bump_action("preview_skip_suicide", active);
                }
                // Play didn't commit. Mark this kind as "tried" so the
                // loop moves on to the other kind instead of infinitely
                // re-trying the same failed pick.
                if picked_is_creature {
                    played_creature = true;
                } else {
                    played_noncreature = true;
                }
            }
            }
        }

        while state.phase != Phase::Combat && state.winner.is_none() {
            state.next_phase();
        }
        if state.winner.is_some() {
            if !events.is_empty() {
                log.push(format!("turn {turn} ({active:?}): {}", events.join("; ")));
            }
            break;
        }

        let defender = active.opponent();
        let attackers: Vec<InstanceId> = eligible_attackers(&state, active)
            .into_iter()
            .filter(|atk| is_attack_worth_declaring(&state, atk, defender))
            .collect();
        let mut declared_atk_count = 0u32;
        for atk in &attackers {
            if state
                .declare_attacker(atk, Some(&mut EventContext::new(lua, &mut oracle)))
                .is_ok()
            {
                declared_atk_count += 1;
            }
        }

        if declared_atk_count > 0 {
            state.confirm_attacks().unwrap();
            let assignments = pick_blocks(&state, defender);
            let mut block_count = 0u32;
            for (blk, atk) in &assignments {
                if state
                    .declare_blocker(blk, atk, Some(&mut EventContext::new(lua, &mut oracle)))
                    .is_ok()
                {
                    block_count += 1;
                }
            }
            let outcome = state
                .confirm_blocks(Some(&mut EventContext::new(lua, &mut oracle)))
                .unwrap();
            bump_attacks(&mut stats, active, declared_atk_count);
            bump_milled(&mut stats, defender, outcome.defender_milled_to_exile as u32);
            for death in &outcome.deaths {
                if state.card_pool.get(death).map(|i| i.owner) == Some(PlayerId::A) {
                    stats.a_deaths += 1;
                } else {
                    stats.b_deaths += 1;
                }
            }
            events.push(format!(
                "{declared_atk_count} attackers, {block_count} blockers → mill {}, {} deaths",
                outcome.defender_milled_to_exile,
                outcome.deaths.len()
            ));
        } else if events.is_empty() {
            events.push("no play, no attack".to_string());
        }

        log.push(format!("turn {turn} ({active:?}): {}", events.join("; ")));

        let starting_turn = state.turn;
        while state.turn == starting_turn && state.winner.is_none() {
            state.next_phase();
        }
    }

    stats.turns = state.turn;
    stats.winner = state.winner.unwrap_or(PlayerId::A);
    stats.a_final_board = state.a.board.len() as u32;
    stats.b_final_board = state.b.board.len() as u32;
    stats.a_final_gy = state.a.graveyard.len() as u32;
    stats.b_final_gy = state.b.graveyard.len() as u32;
    stats.event_fires = state.event_fires.clone();
    stats.action_counts = state.action_counts.clone();
    let replay_journal = state.replay_journal.take().unwrap_or_default();
    stats.replay_journal_entries = replay_journal.len() as u64;
    (stats, replay_journal)
}

fn bump_played(stats: &mut GameStats, p: PlayerId) {
    match p {
        PlayerId::A => stats.a_played += 1,
        PlayerId::B => stats.b_played += 1,
    }
}

fn bump_attacks(stats: &mut GameStats, p: PlayerId, n: u32) {
    match p {
        PlayerId::A => stats.a_attacks += n,
        PlayerId::B => stats.b_attacks += n,
    }
}

fn bump_milled(stats: &mut GameStats, defender: PlayerId, n: u32) {
    match defender {
        PlayerId::A => stats.a_milled_to_exile += n,
        PlayerId::B => stats.b_milled_to_exile += n,
    }
}

fn bump_preview_attempt(stats: &mut GameStats, p: PlayerId, journal_size: u64) {
    match p {
        PlayerId::A => {
            stats.a_preview_attempts += 1;
            stats.a_preview_journal_size_total += journal_size;
        }
        PlayerId::B => {
            stats.b_preview_attempts += 1;
            stats.b_preview_journal_size_total += journal_size;
        }
    }
}

fn bump_preview_rollback(stats: &mut GameStats, p: PlayerId) {
    match p {
        PlayerId::A => stats.a_preview_rollbacks += 1,
        PlayerId::B => stats.b_preview_rollbacks += 1,
    }
}

/// Filter for which kinds the picker is allowed to return. `Any` allows
/// either; `CreatureOnly` / `NonCreatureOnly` constrain to one kind. Used
/// by the multi-card-per-turn loop in run_game to enforce "at most one
/// creature + one non-creature per turn."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickKindFilter {
    Any,
    CreatureOnly,
    NonCreatureOnly,
}

fn pick_random_playable_in_hand(
    state: &GameState,
    player: PlayerId,
    rng: &mut impl Rng,
    kind_filter: PickKindFilter,
) -> Option<InstanceId> {
    let candidates: Vec<&InstanceId> = state
        .player(player)
        .hand
        .iter()
        .filter(|iid| {
            let Some(inst) = state.card_pool.get(*iid) else {
                return false;
            };
            let is_creature = inst.card.kind == CardType::Creature;
            match kind_filter {
                PickKindFilter::Any => {}
                PickKindFilter::CreatureOnly if !is_creature => return false,
                PickKindFilter::NonCreatureOnly if is_creature => return false,
                _ => {}
            }
            match inst.card.kind {
                // Creatures get rigged free + haste before play, so always pickable.
                CardType::Creature => true,
                // Spell (instant or sorcery timing) — main-phase loop here,
                // so sorcery timing is legal. can_pay_instant_cost is
                // shape-equivalent for both timings (HAND/MILL/GRAVEYARD
                // cost rules don't differ).
                CardType::Spell => can_pay_instant_cost(state, player, iid),
                _ => false,
            }
        })
        .collect();
    candidates.choose(rng).map(|iid| (*iid).clone())
}

fn can_pay_instant_cost(state: &GameState, player: PlayerId, iid: &InstanceId) -> bool {
    let Some(inst) = state.card_pool.get(iid) else {
        return false;
    };
    let mut hand_need = 0usize;
    let mut mill_need = 0usize;
    let mut gy_need = 0usize;
    for c in &inst.card.cost {
        if c.is_x {
            return false;
        }
        let amount = c.amount.max(0) as usize;
        match c.source {
            CostSource::Hand => hand_need += amount,
            CostSource::Mill => mill_need += amount,
            CostSource::Graveyard => gy_need += amount,
            _ => return false,
        }
    }
    let p = state.player(player);
    // Subtract 1 for the card being played (it's also in hand).
    let hand_have = p.hand.len().saturating_sub(1);
    hand_have >= hand_need && p.deck.len() >= mill_need && p.graveyard.len() >= gy_need
}

/// Sim heuristic: skip an attack iff the defender has at least one legal
/// blocker AND no legal blocker dies to this attacker's effective X (and the
/// attacker isn't unblockable). When all conceivable blocks leave the blocker
/// alive AND the attack can't reach the player, declaring is strictly bad —
/// the attacker would die or take damage for no gain.
fn is_attack_worth_declaring(
    state: &GameState,
    attacker: &InstanceId,
    defender: PlayerId,
) -> bool {
    let Some(atk_inst) = state.card_pool.get(attacker) else {
        return false;
    };
    if atk_inst.has_keyword("unblockable") {
        return true;
    }
    let atk_x = state.effective_stats(attacker).0;
    let atk_flying = atk_inst.has_keyword("flying");

    let mut any_legal_blocker = false;
    let mut any_kill_possible = false;
    for blk_iid in &state.player(defender).board {
        let Some(blk_inst) = state.card_pool.get(blk_iid) else {
            continue;
        };
        if blk_inst.tapped {
            continue;
        }
        // B.11: flying attacker requires flying blocker.
        if atk_flying && !blk_inst.has_keyword("flying") {
            continue;
        }
        any_legal_blocker = true;
        let blk_y = state.effective_stats(blk_iid).1;
        if atk_x >= blk_y {
            any_kill_possible = true;
            break;
        }
    }

    !any_legal_blocker || any_kill_possible
}

fn eligible_attackers(state: &GameState, player: PlayerId) -> Vec<InstanceId> {
    state
        .player(player)
        .board
        .iter()
        .filter(|iid| {
            let Some(inst) = state.card_pool.get(*iid) else {
                return false;
            };
            if inst.tapped {
                return false;
            }
            if inst.has_keyword("defender") {
                return false;
            }
            if inst.summoning_sick && !inst.has_keyword("haste") {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

fn eligible_blockers(state: &GameState, player: PlayerId) -> Vec<InstanceId> {
    state
        .player(player)
        .board
        .iter()
        .filter(|iid| {
            let Some(inst) = state.card_pool.get(*iid) else {
                return false;
            };
            !inst.tapped && !inst.has_keyword("cannot-block")
        })
        .cloned()
        .collect()
}

/// Tiered block policy (replaces the round-robin distribution). For each
/// declared attacker, sorted by power descending:
///   T3 — clean kill: a blocker with X >= attacker.Y AND Y > attacker.X.
///        Attacker dies, blocker survives. Always take.
///   T2 — kill-trade: a blocker with X >= attacker.Y. Attacker dies, blocker
///        may die. Take if surviving requires it OR if trading for a
///        meaningful threat (attacker.X >= 2).
///   T1 — chump: any blocker, just to absorb damage. Take only if remaining
///        unblocked damage would still deck me.
/// Otherwise let the attacker through (preserve the blocker on board).
///
/// Each blocker is used at most once across the assignment.
fn pick_blocks(
    state: &GameState,
    defender: PlayerId,
) -> Vec<(InstanceId, InstanceId)> {
    use std::collections::BTreeSet;
    use tsot::game::CombatState;

    let declared: Vec<InstanceId> = match &state.combat {
        Some(CombatState::AwaitingBlockers { attacks }) => {
            attacks.iter().map(|a| a.attacker.clone()).collect()
        }
        _ => return Vec::new(),
    };
    if declared.is_empty() {
        return Vec::new();
    }

    let blockers = eligible_blockers(state, defender);
    if blockers.is_empty() {
        return Vec::new();
    }

    // Total incoming if nothing is blocked.
    let total_incoming: i32 = declared
        .iter()
        .map(|a| state.effective_stats(a).0.max(0))
        .sum();
    let deck = state.player(defender).deck.len() as i32;
    let dying = total_incoming >= deck;

    // Sort attackers by power desc — biggest threat handled first.
    let mut sorted: Vec<(InstanceId, i32, i32)> = declared
        .iter()
        .map(|a| {
            let (x, y) = state.effective_stats(a);
            (a.clone(), x, y)
        })
        .collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));

    let mut assignments: Vec<(InstanceId, InstanceId)> = Vec::new();
    let mut used: BTreeSet<InstanceId> = BTreeSet::new();
    let mut remaining_incoming = total_incoming;

    for (atk, atk_x, atk_y) in &sorted {
        let avail: Vec<(InstanceId, i32, i32)> = blockers
            .iter()
            .filter(|b| !used.contains(*b))
            .map(|b| {
                let (x, y) = state.effective_stats(b);
                (b.clone(), x, y)
            })
            .collect();
        if avail.is_empty() {
            break;
        }

        // T3: clean kill — blocker.X >= atk.Y AND blocker.Y > atk.X.
        // Prefer the smallest qualifying blocker (save bigger ones for bigger threats).
        let clean_kill = avail
            .iter()
            .filter(|(_, bx, by)| *bx >= *atk_y && *by > *atk_x)
            .min_by_key(|(_, bx, _)| *bx)
            .cloned();
        if let Some((blk, _, _)) = clean_kill {
            assignments.push((blk.clone(), atk.clone()));
            used.insert(blk);
            remaining_incoming -= atk_x;
            continue;
        }

        // T2: kill-trade — blocker.X >= atk.Y. Take if dying OR atk_x >= 2.
        let kill_trade = avail
            .iter()
            .filter(|(_, bx, _)| *bx >= *atk_y)
            .min_by_key(|(_, bx, _)| *bx)
            .cloned();
        if let Some((blk, _, _)) = kill_trade {
            if dying || *atk_x >= 2 {
                assignments.push((blk.clone(), atk.clone()));
                used.insert(blk);
                remaining_incoming -= atk_x;
                continue;
            }
        }

        // T4: multi-block — pile blockers until combined X >= atk.Y to kill
        // the attacker. Only worth the cost (multiple blockers taking atk.X
        // damage each) when surviving requires removing this attacker.
        // Greedy: biggest-X blockers first so we minimize how many we commit.
        if dying {
            let mut by_x = avail.clone();
            by_x.sort_by_key(|(_, bx, _)| std::cmp::Reverse(*bx));
            let mut combined_x = 0i32;
            let mut picks: Vec<InstanceId> = Vec::new();
            for (b, bx, _) in &by_x {
                if combined_x >= *atk_y {
                    break;
                }
                combined_x += *bx;
                picks.push(b.clone());
            }
            if combined_x >= *atk_y && picks.len() >= 2 {
                for blk in picks {
                    assignments.push((blk.clone(), atk.clone()));
                    used.insert(blk);
                }
                remaining_incoming -= atk_x;
                continue;
            }
        }

        // T1: chump — only if I'm still dying after assignments so far.
        // Prefer the smallest available blocker (preserve big ones).
        if remaining_incoming >= deck {
            let chump = avail.iter().min_by_key(|(_, bx, _)| *bx).cloned();
            if let Some((blk, _, _)) = chump {
                assignments.push((blk.clone(), atk.clone()));
                used.insert(blk);
                remaining_incoming -= atk_x;
                continue;
            }
        }

        // Otherwise let this attacker through.
    }

    assignments
}

fn rig_creature_free_haste(state: &mut GameState, iid: &InstanceId) {
    let inst = state.card_pool.get_mut(iid).unwrap();
    inst.card.cost = vec![];
    inst.card.abilities.push("haste".to_string());
}

fn short(iid: &InstanceId) -> String {
    let parts: Vec<&str> = iid.splitn(3, ':').collect();
    if parts.len() == 3 {
        format!("{}:{}", parts[0], parts[2])
    } else {
        iid.clone()
    }
}

fn print_aggregate(all: &[GameStats], elapsed: std::time::Duration) {
    let n = all.len() as f64;
    let a_wins = all.iter().filter(|s| s.winner == PlayerId::A).count();
    let b_wins = all.iter().filter(|s| s.winner == PlayerId::B).count();

    let mut turn_values: Vec<u32> = all.iter().map(|s| s.turns).collect();
    turn_values.sort_unstable();
    let turn_min = turn_values.first().copied().unwrap_or(0);
    let turn_max = turn_values.last().copied().unwrap_or(0);
    let turn_mean: f64 = turn_values.iter().sum::<u32>() as f64 / n;
    let turn_median = turn_values[turn_values.len() / 2];

    println!();
    println!("=== Aggregate over {} games (elapsed {:.2?}, avg {:.1?} per game) ===", all.len(), elapsed, elapsed / all.len() as u32);
    println!();
    println!("Winners:  A {} ({:.0}%)   B {} ({:.0}%)",
        a_wins, 100.0 * a_wins as f64 / n,
        b_wins, 100.0 * b_wins as f64 / n,
    );
    println!();
    println!("Turn count:  min {}   median {}   mean {:.1}   max {}",
        turn_min, turn_median, turn_mean, turn_max);
    println!();
    println!("Per-game averages:");
    println!("                       A           B");
    println!("  cards played        {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_played as f64),
        avg(all, |s| s.b_played as f64));
    println!("  attacks declared    {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_attacks as f64),
        avg(all, |s| s.b_attacks as f64));
    println!("  deaths (own creat.) {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_deaths as f64),
        avg(all, |s| s.b_deaths as f64));
    println!("  milled to exile     {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_milled_to_exile as f64),
        avg(all, |s| s.b_milled_to_exile as f64));
    println!("  final board size    {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_final_board as f64),
        avg(all, |s| s.b_final_board as f64));
    println!("  final graveyard     {:>6.1}      {:>6.1}",
        avg(all, |s| s.a_final_gy as f64),
        avg(all, |s| s.b_final_gy as f64));

    println!();
    println!("Event firing breakdown (per-game averages, A.1 triggered abilities):");
    println!("                          A         B    wired");
    for ev in EventName::ALL {
        let a_avg = avg(all, |s| s.event_fires.get(&ev).map(|v| v[0]).unwrap_or(0) as f64);
        let b_avg = avg(all, |s| s.event_fires.get(&ev).map(|v| v[1]).unwrap_or(0) as f64);
        let any_fired = all
            .iter()
            .any(|s| s.event_fires.get(&ev).is_some_and(|v| v[0] + v[1] > 0));
        let marker = if any_fired { "yes" } else { " no" };
        println!("  {:20} {:>6.2}    {:>6.2}    {}", ev.lua_key(), a_avg, b_avg, marker);
    }

    println!();
    println!("Engine + handler actions (totals across {} games):", all.len());
    println!("                              A         B");
    for action in [
        "draw",
        "mill",
        "damage",
        "move",
        "discard",
        "tap",
        "untap",
        "add_status",
        "add_modifier",
        "choose_card",
        "choose_player",
        "choose_int",
        "confirm",
        "decked_by_handler_draw",
        "preview_skip_suicide",
        "preview_retry_rescued",
        "counter_top",
        "instant_response_played",
    ] {
        let a_total: u64 = all
            .iter()
            .map(|s| s.action_counts.get(action).map(|v| v[0]).unwrap_or(0) as u64)
            .sum();
        let b_total: u64 = all
            .iter()
            .map(|s| s.action_counts.get(action).map(|v| v[1]).unwrap_or(0) as u64)
            .sum();
        println!("  game.{action:24} {a_total:>6}    {b_total:>6}");
    }

    println!();
    println!("Future-simulation telemetry (per-game averages — every play opens a journal):");
    println!("                          A         B");
    let attempts_a = avg(all, |s| s.a_preview_attempts as f64);
    let attempts_b = avg(all, |s| s.b_preview_attempts as f64);
    println!("  preview attempts      {attempts_a:>6.2}    {attempts_b:>6.2}");
    println!(
        "  rolled back           {:>6.2}    {:>6.2}",
        avg(all, |s| s.a_preview_rollbacks as f64),
        avg(all, |s| s.b_preview_rollbacks as f64)
    );
    println!(
        "  mutations explored    {:>6.1}    {:>6.1}    (sum of journal entries per game)",
        avg(all, |s| s.a_preview_journal_size_total as f64),
        avg(all, |s| s.b_preview_journal_size_total as f64)
    );
    let avg_size_a = if attempts_a > 0.0 {
        avg(all, |s| s.a_preview_journal_size_total as f64) / attempts_a
    } else {
        0.0
    };
    let avg_size_b = if attempts_b > 0.0 {
        avg(all, |s| s.b_preview_journal_size_total as f64) / attempts_b
    } else {
        0.0
    };
    println!(
        "  avg mutations / play  {avg_size_a:>6.2}    {avg_size_b:>6.2}    (depth of each previewed future)"
    );
    let replay_avg = avg(all, |s| s.replay_journal_entries as f64);
    let replay_min = all
        .iter()
        .map(|s| s.replay_journal_entries)
        .min()
        .unwrap_or(0);
    let replay_max = all
        .iter()
        .map(|s| s.replay_journal_entries)
        .max()
        .unwrap_or(0);
    println!();
    println!(
        "Replay journal (per game, captures every committed mutation from start to game-end):"
    );
    println!("  entries   avg {replay_avg:>6.1}   min {replay_min:>4}   max {replay_max:>4}");

    println!();
    println!("Pending mechanics (zero today; nonzero once each engine piece lands):");
    println!("                                  A         B");
    print_pending("sacrifices (cost P.16)");
    print_pending("activated abilities used");
    // Instant responses now wired via piece 4 — read from action_counts.
    let resp_a: f64 = all
        .iter()
        .map(|s| {
            s.action_counts
                .get("instant_response_played")
                .map(|v| v[0] as f64)
                .unwrap_or(0.0)
        })
        .sum::<f64>()
        / all.len() as f64;
    let resp_b: f64 = all
        .iter()
        .map(|s| {
            s.action_counts
                .get("instant_response_played")
                .map(|v| v[1] as f64)
                .unwrap_or(0.0)
        })
        .sum::<f64>()
        / all.len() as f64;
    println!(
        "  {:35} {:>6.2}    {:>6.2}",
        "instant responses (R.1)", resp_a, resp_b
    );
    print_pending("artifacts played (P.19)");
    print_pending("environments played (P.21)");
    print_pending("mulligans (S.2/S.3)");
    print_pending("counters on the stack");
    print_pending("color/symbol/type mutations");

    println!();
    println!("Matchup matrix (cell = A-side win rate; n = games in that pairing):");
    print!("           ");
    for v in &VARIANTS {
        print!("  B:{}    ", variant_label(*v));
    }
    println!();
    for va in &VARIANTS {
        print!("  A:{}     ", variant_label(*va));
        for vb in &VARIANTS {
            let games: Vec<&GameStats> = all
                .iter()
                .filter(|s| s.variant_a == *va && s.variant_b == *vb)
                .collect();
            if games.is_empty() {
                print!("  --  ({:>2})", 0);
                continue;
            }
            let wins = games.iter().filter(|s| s.winner == PlayerId::A).count();
            let rate = wins as f64 / games.len() as f64;
            print!(" {:>4.2} ({:>3})", rate, games.len());
        }
        println!();
    }

    println!();
    println!("Per-variant aggregate win rate (across all opponents, both sides):");
    println!("  Variant   games   wins   rate");
    for v in &VARIANTS {
        let mut games = 0u32;
        let mut wins = 0u32;
        for s in all {
            if s.variant_a == *v {
                games += 1;
                if s.winner == PlayerId::A {
                    wins += 1;
                }
            }
            if s.variant_b == *v {
                games += 1;
                if s.winner == PlayerId::B {
                    wins += 1;
                }
            }
        }
        let rate = if games > 0 {
            wins as f64 / games as f64
        } else {
            0.0
        };
        println!("  {}        {:>5}   {:>4}   {:.2}", variant_label(*v), games, wins, rate);
    }
}

fn print_pending(label: &str) {
    println!("  {label:35} {:>6.1}    {:>6.1}", 0.0_f64, 0.0_f64);
}

fn avg<F: Fn(&GameStats) -> f64>(all: &[GameStats], f: F) -> f64 {
    all.iter().map(f).sum::<f64>() / all.len() as f64
}
