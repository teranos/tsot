use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::HashMap;
use std::path::Path;
use tsot::card::{Card, CardRegistry, CardType, EventName};
use tsot::game::{GameState, InstanceId, Phase, PlayChoices, PlayerId};

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
    event_fires: HashMap<EventName, [u32; 2]>,
}

fn main() -> mlua::Result<()> {
    let registry = CardRegistry::load(Path::new("cards"))?;
    // Filter to standard-legal creatures (per S.5, the `test` subtype is excluded).
    let creature_pool: Vec<Card> = registry
        .cards()
        .iter()
        .filter(|c| matches!(c.kind, CardType::Creature))
        .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
        .cloned()
        .collect();

    println!(
        "loaded {} cards ({} standard-legal creatures); running {} simulations",
        registry.cards().len(),
        creature_pool.len(),
        ITERATIONS
    );

    let mut rng = rand::thread_rng();
    let mut all: Vec<GameStats> = Vec::with_capacity(ITERATIONS);
    let mut last_log: Vec<String> = Vec::new();

    let t0 = std::time::Instant::now();
    for _ in 0..ITERATIONS {
        let deck_a = build_random_deck(&creature_pool, &mut rng, 50);
        let deck_b = build_random_deck(&creature_pool, &mut rng, 50);
        let state = GameState::new(deck_a, deck_b);
        last_log.clear();
        all.push(run_game(state, &mut rng, &mut last_log, registry.lua()));
    }
    let elapsed = t0.elapsed();

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
    rng: &mut impl Rng,
    log: &mut Vec<String>,
    lua: &mlua::Lua,
) -> GameStats {
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
        event_fires: HashMap::new(),
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

        if let Some(creature) = pick_random_creature_in_hand(&state, active, rng) {
            rig_creature_free_haste(&mut state, &creature);
            if state.play_card(active, &creature, PlayChoices::default()).is_ok() {
                bump_played(&mut stats, active);
                let (x, y) = state.effective_stats(&creature);
                events.push(format!("played {} ({x}/{y})", short(&creature)));
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

        let attackers = eligible_attackers(&state, active);
        let mut declared_atk_count = 0u32;
        for atk in &attackers {
            if state.declare_attacker(atk).is_ok() {
                declared_atk_count += 1;
            }
        }

        if declared_atk_count > 0 {
            state.confirm_attacks().unwrap();
            let defender = active.opponent();
            let blockers = eligible_blockers(&state, defender);
            let mut block_count = 0u32;
            if !attackers.is_empty() {
                for (i, blk) in blockers.iter().enumerate() {
                    let atk = &attackers[i % attackers.len()];
                    if state.declare_blocker(blk, atk, Some(lua)).is_ok() {
                        block_count += 1;
                    }
                }
            }
            let outcome = state.confirm_blocks(Some(lua)).unwrap();
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
    stats
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

fn pick_random_creature_in_hand(
    state: &GameState,
    player: PlayerId,
    rng: &mut impl Rng,
) -> Option<InstanceId> {
    let creatures: Vec<&InstanceId> = state
        .player(player)
        .hand
        .iter()
        .filter(|iid| {
            matches!(
                state.card_pool.get(*iid).map(|c| c.card.kind),
                Some(CardType::Creature)
            )
        })
        .collect();
    creatures.choose(rng).map(|iid| (*iid).clone())
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
    println!("Pending mechanics (zero today; nonzero once each engine piece lands):");
    println!("                                  A         B");
    print_pending("draws from effects (A.4)");
    print_pending("discards (HAND → GRAVEYARD)");
    print_pending("sacrifices (cost P.16)");
    print_pending("bounces (BOARD → HAND)");
    print_pending("activated abilities used");
    print_pending("attached cards on board (P.6)");
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
