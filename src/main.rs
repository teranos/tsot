use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::collections::BTreeMap;
use std::path::Path;
use tsot::card::{Card, CardRegistry, CardType, CostSource, EventName};
use tsot::choice::RandomOracle;
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

const ITERATIONS: usize = 1000;

#[derive(Debug, Clone)]
struct GameStats {
    turns: u32,
    winner: PlayerId,
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
        .filter(|c| matches!(c.kind, CardType::Creature | CardType::Instant))
        .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
        .filter(|c| {
            c.cost.iter().all(|cc| {
                !cc.is_x
                    && matches!(
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
        .filter(|c| matches!(c.kind, CardType::Instant))
        .count();

    println!(
        "loaded {} cards ({} creatures + {} instants in deck pool); running {} simulations",
        registry.cards().len(),
        creature_count,
        instant_count,
        ITERATIONS
    );

    let seed = pick_seed();
    println!("seed: {seed}");
    let mut rng = StdRng::seed_from_u64(seed);
    let mut all: Vec<GameStats> = Vec::with_capacity(ITERATIONS);
    let mut last_log: Vec<String> = Vec::new();

    let replay_out_path = std::env::var("TSOT_REPLAY_OUT").ok();

    let t0 = std::time::Instant::now();
    let mut last_deck_a_ids: Vec<String> = Vec::new();
    let mut last_deck_b_ids: Vec<String> = Vec::new();
    let mut last_journal: tsot::game::Journal = tsot::game::Journal::new();
    for _ in 0..ITERATIONS {
        let deck_a = build_random_deck(&playable_pool, &mut rng, 50);
        let deck_b = build_random_deck(&playable_pool, &mut rng, 50);
        last_deck_a_ids = deck_a.iter().map(|c| c.id.clone()).collect();
        last_deck_b_ids = deck_b.iter().map(|c| c.id.clone()).collect();
        let state = GameState::new(deck_a, deck_b);
        last_log.clear();
        let (stats, journal) = run_game(state, &mut rng, &mut last_log, registry.lua());
        all.push(stats);
        last_journal = journal;
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
    let mut oracle = RandomOracle::new(StdRng::seed_from_u64(oracle_seed));

    // Open a game-long replay journal. Every committed mutation will be
    // recorded into this for the duration of the game; previewed-and-skipped
    // mutations stay isolated in the per-action `state.journal`.
    state.replay_journal = Some(tsot::game::Journal::new());
    let mut stats = GameStats {
        turns: 0,
        winner: PlayerId::A,
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

        if let Some(picked) = pick_random_playable_in_hand(&state, active, rng) {
            let kind = state
                .card_pool
                .get(&picked)
                .map(|c| c.card.kind)
                .unwrap_or(CardType::Unspecified);
            let mut choices = PlayChoices::default();
            if matches!(kind, CardType::Creature) {
                rig_creature_free_haste(&mut state, &picked);
            } else if matches!(kind, CardType::Instant) {
                // Pay HAND cost honestly: take the leftmost N hand cards
                // (excluding the card being played). Deterministic discard
                // pending choice API.
                let cost = state
                    .card_pool
                    .get(&picked)
                    .map(|c| c.card.cost.clone())
                    .unwrap_or_default();
                let hand_needed: usize = cost
                    .iter()
                    .filter(|c| matches!(c.source, CostSource::Hand))
                    .map(|c| c.amount.max(0) as usize)
                    .sum();
                if hand_needed > 0 {
                    let payment: Vec<InstanceId> = state
                        .player(active)
                        .hand
                        .iter()
                        .filter(|iid| *iid != &picked)
                        .take(hand_needed)
                        .cloned()
                        .collect();
                    choices.hand_payment_ids = payment;
                }
            }
            // Preview-and-skip: open a journal, attempt the play. If the
            // play would deck the active player (suicide), rollback and
            // skip. Otherwise discard the journal and keep the mutations.
            state.journal = Some(tsot::game::Journal::new());
            let opponent_of_active = active.opponent();
            let result = state.play_card(
                active,
                &picked,
                choices,
                Some(&mut EventContext::new(lua, &mut oracle)),
            );
            let suicide = state.winner == Some(opponent_of_active);
            let preview_size = state.journal.as_ref().map(|j| j.len()).unwrap_or(0) as u64;

            // Telemetry: every play opened a journal, so we count it.
            bump_preview_attempt(&mut stats, active, preview_size);

            if result.is_ok() && !suicide {
                // Commit: transfer preview entries into the replay journal,
                // then drop the preview.
                if let Some(mut preview) = state.journal.take() {
                    if let Some(replay) = state.replay_journal.as_mut() {
                        replay.extend_from(&mut preview);
                    }
                }
                bump_played(&mut stats, active);
                let label = match kind {
                    CardType::Instant => format!("instant {}", short(&picked)),
                    _ => {
                        let (x, y) = state.effective_stats(&picked);
                        format!("{} ({x}/{y})", short(&picked))
                    }
                };
                events.push(format!("played {label}"));
            } else {
                if let Some(journal) = state.journal.take() {
                    journal.rollback(&mut state);
                }
                bump_preview_rollback(&mut stats, active);
                if suicide {
                    state.bump_action("preview_skip_suicide", active);
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
            let blockers = eligible_blockers(&state, defender);
            let mut block_count = 0u32;
            if !attackers.is_empty() {
                for (i, blk) in blockers.iter().enumerate() {
                    let atk = &attackers[i % attackers.len()];
                    if state
                        .declare_blocker(blk, atk, Some(&mut EventContext::new(lua, &mut oracle)))
                        .is_ok()
                    {
                        block_count += 1;
                    }
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

fn pick_random_playable_in_hand(
    state: &GameState,
    player: PlayerId,
    rng: &mut impl Rng,
) -> Option<InstanceId> {
    let candidates: Vec<&InstanceId> = state
        .player(player)
        .hand
        .iter()
        .filter(|iid| {
            let Some(inst) = state.card_pool.get(*iid) else {
                return false;
            };
            match inst.card.kind {
                // Creatures get rigged free + haste before play, so always pickable.
                CardType::Creature => true,
                CardType::Instant => can_pay_instant_cost(state, player, iid),
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
            !inst.tapped
        })
        .cloned()
        .collect()
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
    println!("Engine + handler actions (per-game averages):");
    println!("                          A         B");
    for action in [
        "draw",
        "mill",
        "damage",
        "move",
        "discard",
        "tap",
        "untap",
        "add_status",
        "choose_card",
        "confirm",
        "self_deckout_by_choice",
        "preview_skip_suicide",
    ] {
        let a_avg = avg(all, |s| s.action_counts.get(action).map(|v| v[0]).unwrap_or(0) as f64);
        let b_avg = avg(all, |s| s.action_counts.get(action).map(|v| v[1]).unwrap_or(0) as f64);
        println!("  game.{action:16} {a_avg:>6.2}    {b_avg:>6.2}");
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
    print_pending("instant responses (R.1)");
    print_pending("artifacts played (P.19)");
    print_pending("environments played (P.21)");
    print_pending("mulligans (S.2/S.3)");
    print_pending("counters on the stack");
    print_pending("color/symbol/type mutations");
}

fn print_pending(label: &str) {
    println!("  {label:35} {:>6.1}    {:>6.1}", 0.0_f64, 0.0_f64);
}

fn avg<F: Fn(&GameStats) -> f64>(all: &[GameStats], f: F) -> f64 {
    all.iter().map(f).sum::<f64>() / all.len() as f64
}
