//! Per-game turn loop. Calls into [`super::ai`] for AI decisions, writes
//! into [`super::stats::GameStats`] as the game progresses, returns the
//! final stats + the game-long replay journal.

use std::collections::{BTreeMap, BTreeSet};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::{CardType, CostSource};
use tsot::choice::{ChoiceOracle, ChooseIntRequest, RandomOracle, RecordingOracle, ScriptedOracle};
use tsot::game::{EventContext, GameState, InstanceId, Phase, PlayChoices, PlayerId};

use super::ai::{
    eligible_attackers, is_attack_worth_declaring, pick_blocks, pick_random_playable_in_hand,
    rig_creature_free_haste, sacrifice_keep_value, PickKindFilter,
};
use super::stats::{
    bump_attacks, bump_milled, bump_played, bump_preview_attempt, bump_preview_rollback, GameStats,
};
use super::variants::DeckVariant;

pub fn run_game(
    mut state: GameState,
    rng: &mut StdRng,
    log: &mut Vec<String>,
    lua: &mlua::Lua,
) -> (GameStats, tsot::game::Journal) {
    let oracle_seed: u64 = rng.gen();
    let mut oracle = RecordingOracle::new(RandomOracle::new(StdRng::seed_from_u64(oracle_seed)));

    state.replay_journal = Some(tsot::game::Journal::new());
    let mut stats = GameStats {
        turns: 0,
        winner: PlayerId::A,
        variant_a: DeckVariant::Ra,
        variant_b: DeckVariant::Rb,
        token_a: String::new(),
        token_b: String::new(),
        game_index: 0,
        deck_a_ids: BTreeSet::new(),
        deck_b_ids: BTreeSet::new(),
        a_played_card_ids: BTreeSet::new(),
        b_played_card_ids: BTreeSet::new(),
        card_play_turns: BTreeMap::new(),
        card_sacrificed_count: BTreeMap::new(),
        card_discarded_count: BTreeMap::new(),
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

        while state.phase != Phase::Main1 && state.winner.is_none() {
            state.next_phase();
        }
        if state.winner.is_some() {
            log.push(format!("turn {turn} ({active:?}): deck-out before Main1"));
            break;
        }

        // Multi-card-per-turn (Pattern B): at most one creature per turn,
        // but as many non-creatures as the AI can afford.
        let mut played_creature = false;
        loop {
            if state.winner.is_some() {
                break;
            }
            let kind_filter = if played_creature {
                PickKindFilter::NonCreatureOnly
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
            {
                let kind = state
                    .card_pool
                    .get(&picked)
                    .map(|c| c.card.kind)
                    .unwrap_or(CardType::Unspecified);
                let mut choices = PlayChoices::default();
                let cost = state
                    .card_pool
                    .get(&picked)
                    .map(|c| c.card.cost.clone())
                    .unwrap_or_default();
                let has_is_x = cost.iter().any(|c| c.is_x);

                if has_is_x {
                    let hand_size = state.player(active).hand.len();
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
                        choices.hand_payment_ids = state.resolve_hand_payment(
                            active,
                            &picked,
                            hand_needed,
                            &mut oracle,
                        );
                    }
                } else if matches!(kind, CardType::Creature) {
                    let has_setup_cost = cost.iter().any(|c| {
                        matches!(c.source, CostSource::Sacrifice | CostSource::Graveyard)
                    });
                    if !has_setup_cost {
                        rig_creature_free_haste(&mut state, &picked);
                    }
                } else if matches!(
                    kind,
                    CardType::Spell | CardType::Artifact | CardType::Mutation
                ) {
                    let raw_hand_needed: usize = cost
                        .iter()
                        .filter(|c| matches!(c.source, CostSource::Hand))
                        .map(|c| c.amount.max(0) as usize)
                        .sum();
                    let hand_red = state
                        .cost_reduction(&picked, CostSource::Hand)
                        .max(0) as usize;
                    let mut hand_needed = raw_hand_needed.saturating_sub(hand_red);
                    if hand_needed > 0 {
                        if let Some(jewel) = state.find_jewel_tap_candidate(active, &picked) {
                            choices.jewel_tap = Some(jewel);
                            hand_needed -= 1;
                        }
                    }
                    if hand_needed > 0 {
                        choices.hand_payment_ids = state.resolve_hand_payment(
                            active,
                            &picked,
                            hand_needed,
                            &mut oracle,
                        );
                    }
                    if matches!(kind, CardType::Mutation) {
                        let mut pool: Vec<InstanceId> = state
                            .a
                            .board
                            .iter()
                            .chain(state.b.board.iter())
                            .filter(|t| {
                                state
                                    .card_pool
                                    .get(*t)
                                    .map(|i| i.card.kind == CardType::Creature)
                                    .unwrap_or(false)
                            })
                            .cloned()
                            .collect();
                        pool.sort_by_key(|t| {
                            let inst = state.card_pool.get(t);
                            let own = inst.map(|i| i.controller == active).unwrap_or(false);
                            let x = state.effective_stats(t).0;
                            (if own { 0 } else { 1 }, -x)
                        });
                        choices.mutation_target = pool.first().cloned();
                    }
                }
                let sacrifice_slots: Vec<Option<CardType>> = cost
                    .iter()
                    .filter(|c| matches!(c.source, CostSource::Sacrifice))
                    .flat_map(|c| {
                        let n = c.amount.max(0) as usize;
                        std::iter::repeat_n(c.kind, n)
                    })
                    .collect();
                if !sacrifice_slots.is_empty() {
                    let mut used: std::collections::BTreeSet<InstanceId> =
                        std::collections::BTreeSet::new();
                    for required_kind in sacrifice_slots {
                        let mut sac_candidates: Vec<InstanceId> = state
                            .player(active)
                            .board
                            .iter()
                            .filter(|iid| !used.contains(*iid))
                            .filter(|iid| {
                                if let Some(k) = required_kind {
                                    state
                                        .card_pool
                                        .get(*iid)
                                        .map(|i| i.card.kind == k)
                                        .unwrap_or(false)
                                } else {
                                    true
                                }
                            })
                            .cloned()
                            .collect();
                        sac_candidates.sort_by_key(|iid| sacrifice_keep_value(&state, iid));
                        if let Some(pick) = sac_candidates.into_iter().next() {
                            if let Some(card_id) =
                                state.card_pool.get(&pick).map(|c| c.card.id.clone())
                            {
                                *stats.card_sacrificed_count.entry(card_id).or_insert(0) += 1;
                            }
                            used.insert(pick.clone());
                            choices.sacrifice_ids.push(pick);
                        }
                    }
                }

                oracle.clear();
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

                bump_preview_attempt(&mut stats, active, preview_size);

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
                    if let Some(mut preview) = state.journal.take() {
                        if let Some(replay) = state.replay_journal.as_mut() {
                            replay.extend_from(&mut preview);
                        }
                    }
                    bump_played(&mut stats, active);
                    if let Some(card_id) =
                        state.card_pool.get(&picked).map(|c| c.card.id.clone())
                    {
                        match active {
                            PlayerId::A => {
                                stats.a_played_card_ids.insert(card_id.clone());
                            }
                            PlayerId::B => {
                                stats.b_played_card_ids.insert(card_id.clone());
                            }
                        }
                        let turn_now = state.turn;
                        stats
                            .card_play_turns
                            .entry(card_id)
                            .and_modify(|(min_t, max_t)| {
                                if turn_now < *min_t {
                                    *min_t = turn_now;
                                }
                                if turn_now > *max_t {
                                    *max_t = turn_now;
                                }
                            })
                            .or_insert((turn_now, turn_now));
                    }
                    let timing = state.card_pool.get(&picked).and_then(|c| c.card.timing);
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
                    }
                } else {
                    if let Some(journal) = state.journal.take() {
                        journal.rollback(&mut state);
                    }
                    bump_preview_rollback(&mut stats, active);
                    if suicide {
                        state.bump_action("preview_skip_suicide", active);
                    }
                    if picked_is_creature {
                        played_creature = true;
                    } else {
                        break;
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
    for (key, counts) in &state.action_counts {
        if let Some(cid) = key.strip_prefix("discarded:") {
            let total = counts[0] + counts[1];
            *stats
                .card_discarded_count
                .entry(cid.to_string())
                .or_insert(0) += total;
        }
    }
    let replay_journal = state.replay_journal.take().unwrap_or_default();
    stats.replay_journal_entries = replay_journal.len() as u64;
    (stats, replay_journal)
}

pub fn short(iid: &InstanceId) -> String {
    let parts: Vec<&str> = iid.splitn(3, ':').collect();
    if parts.len() == 3 {
        format!("{}:{}", parts[0], parts[2])
    } else {
        iid.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsot::card::CardRegistry;

    /// EA prerequisite: fitness(genome) is meaningful only if
    /// `run_game(state, &mut rng, &mut log, lua)` produces byte-identical
    /// outputs for byte-identical inputs. If this ever fails, the EA's
    /// generation-to-generation signal is noise and everything downstream
    /// is chasing it.
    #[test]
    fn run_game_is_deterministic_per_seed_and_decks() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| matches!(c.kind, tsot::card::CardType::Creature))
            .unwrap()
            .clone();
        let deck_a: Vec<tsot::card::Card> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();

        let state_1 = GameState::new(deck_a.clone(), deck_b.clone());
        let state_2 = GameState::new(deck_a, deck_b);
        let mut rng_1 = StdRng::seed_from_u64(0xEA_C8);
        let mut rng_2 = StdRng::seed_from_u64(0xEA_C8);
        let mut log_1: Vec<String> = Vec::new();
        let mut log_2: Vec<String> = Vec::new();

        let (stats_1, journal_1) = run_game(state_1, &mut rng_1, &mut log_1, registry.lua());
        let (stats_2, journal_2) = run_game(state_2, &mut rng_2, &mut log_2, registry.lua());

        assert_eq!(log_1, log_2, "logs diverged across identical runs");
        assert_eq!(
            format!("{stats_1:?}"),
            format!("{stats_2:?}"),
            "GameStats diverged across identical runs"
        );
        assert_eq!(
            format!("{journal_1:?}"),
            format!("{journal_2:?}"),
            "journals diverged across identical runs"
        );
    }
}
