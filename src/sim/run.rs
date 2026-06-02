//! Per-game turn loop. Calls into [`super::ai`] for AI decisions, writes
//! into [`super::stats::GameStats`] as the game progresses, returns the
//! final stats + the game-long replay journal.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use tsot::card::{CardType, CostSource};
use tsot::choice::{ChoiceOracle, ChooseIntRequest, RandomOracle, RecordingOracle, ScriptedOracle};
use tsot::game::{EventContext, GameState, InstanceId, Phase, PlayChoices, PlayerId};

use super::ai::{
    attached_keep_value, pick_blocks, pick_random_playable_in_hand, rig_creature_free_haste,
    sacrifice_keep_value, select_attackers, PickKindFilter,
};
use super::stats::{
    bump_attacks, bump_milled, bump_played, bump_preview_attempt, bump_preview_rollback, GameStats,
};
use super::variants::DeckVariant;

/// Original entry point — takes ownership of state, manages its own
/// `replay_journal`, returns both stats and the captured journal. The
/// `--replay` workflow and existing callers depend on this shape.
///
/// Internally delegates to [`run_game_continue`]. New callers that
/// want to drive multiple games on the same state (MCTS rollouts,
/// scripted multi-game tests, multiplayer rollback) use
/// `run_game_continue` directly with `&mut GameState`.
pub fn run_game(
    state: GameState,
    rng: &mut StdRng,
    log: &mut Vec<String>,
    lua: &mlua::Lua,
) -> (GameStats, tsot::game::Journal) {
    let ais = [super::AiKind::Heuristic, super::AiKind::Heuristic];
    run_game_with_ai(state, rng, log, lua, &ais)
}

/// Like [`run_game`] but with per-player AI selection. Used by the
/// EA when opponents play MCTS (step 8 — `--opponent-ai mcts`) and
/// anywhere else that wants the wrapper's journal-lifecycle setup
/// without being locked to Heuristic-on-both-sides.
pub fn run_game_with_ai(
    mut state: GameState,
    rng: &mut StdRng,
    log: &mut Vec<String>,
    lua: &mlua::Lua,
    ais: &[super::AiKind; 2],
) -> (GameStats, tsot::game::Journal) {
    state.replay_journal = Some(tsot::game::Journal::new());
    let mut stats = run_game_continue(&mut state, rng, log, lua, ais);
    let replay_journal = state.replay_journal.take().unwrap_or_default();
    stats.replay_journal_entries = replay_journal.len() as u64;
    (stats, replay_journal)
}

/// Resumable game loop. Operates on `&mut GameState` without touching
/// the caller's `replay_journal` lifecycle — caller decides whether to
/// open one, whether to take/rollback at the end. Required entry point
/// for journal-based AI search (MCTS) and any scenario where the game
/// loop is one step inside a larger rollback-able operation.
///
/// Preconditions:
///   - `state` is in a runnable position (winner = None, phase + active
///     player consistent). New games via `GameState::new(...)` qualify;
///     mid-game states from a save also qualify.
///   - Caller has set `state.replay_journal` if they want a recording.
///     The function does NOT reset it.
///
/// The returned `GameStats.replay_journal_entries` field is left at
/// `0` — the wrapper that owns the journal lifecycle sets it. Bare
/// callers can set it themselves after taking the journal.
/// Outcome of `build_pattern_b_choices`. Variants beyond `Choices`
/// signal Pattern B should special-case (skip the play, advance the
/// loop, etc.).
pub(crate) enum BuildChoiceResult {
    /// PlayChoices ready to feed to `state.play_card`.
    Choices(PlayChoices),
    /// Picked an X-cost card and the X-pick computed max_x < 1 (no
    /// affordable X ≥ 1). Caller should advance Pattern B: skip the
    /// play, and mark `played_creature` to prevent re-picking if
    /// the candidate was a creature.
    UnaffordableX { picked_is_creature: bool },
}

/// Build the same `PlayChoices` Pattern B builds inline today —
/// extracted so MCTS rollouts construct choices identically to the
/// heuristic AI (rather than its own simpler version that was
/// systematically underestimating candidates with non-trivial cost).
///
/// Mirrors the inline logic exactly:
///   - X-cost: cap by tightest resource, oracle picks X, resolve hand
///     payment (smart-pitch via oracle) + GY substitutes + GY-pay
///   - Creature: hand payment + GY payment + rig_creature_free_haste
///     shortcut when no setup cost
///   - Spell / Artifact / Mutation: hand payment + GY pay + mutation
///     target selection
///   - Sacrifice slots (any card kind): low-value picker
///   - P.31 ATTACHED-source slots
///
/// The function mutates `state` for the rig + sacrifice picking +
/// activations are not journaled directly here; the journaled
/// helpers (set_*, move_card, etc.) handle that, and rig_creature_
/// free_haste is journaled via its own variant. So MCTS rollouts can
/// rollback the entire build_pattern_b_choices + play_card sequence.
///
/// `active_is_human` controls whether the AI-side `rig_creature_free_haste`
/// shortcut applies. That shortcut clears the cast cost and grants haste
/// to non-setup-cost creatures so the AI can swing same-turn — desirable
/// for fitness sims, but it permanently mutates the card and breaks the
/// rules for a human player. Pass `false` for AI sides (default behavior),
/// `true` to skip the rig (human play).
pub(crate) fn build_pattern_b_choices(
    state: &mut GameState,
    active: PlayerId,
    picked: &InstanceId,
    oracle: &mut dyn ChoiceOracle,
    active_is_human: bool,
) -> BuildChoiceResult {
    let kind = state
        .card_pool
        .get(picked)
        .map(|c| c.card.kind)
        .unwrap_or(CardType::Unspecified);
    let picked_is_creature = matches!(kind, CardType::Creature);
    let mut choices = PlayChoices::default();
    let cost = state
        .card_pool
        .get(picked)
        .map(|c| c.card.cost.clone())
        .unwrap_or_default();
    let has_is_x = cost.iter().any(|c| c.is_x);

    if has_is_x {
        let p = state.player(active);
        let hand_size = p.hand.len();
        let deck_size = p.deck.len();
        let gy_size = p.graveyard.len();
        let board_creatures = p
            .board
            .iter()
            .filter(|iid| {
                state
                    .card_pool
                    .get(*iid)
                    .map(|i| i.card.kind == CardType::Creature)
                    .unwrap_or(false)
            })
            .count();
        let identity_count = state.identity_matching_hand_count(active, picked);
        let gy_subs_available = p
            .graveyard
            .iter()
            .filter(|gid| {
                state
                    .card_pool
                    .get(*gid)
                    .map(|i| i.card.gy_hand_substitute)
                    .unwrap_or(false)
            })
            .count();
        let mut caps: Vec<usize> = Vec::new();
        for c in &cost {
            if !c.is_x {
                continue;
            }
            match c.source {
                CostSource::Hand => {
                    let hand_avail = identity_count.min(hand_size.saturating_sub(1));
                    caps.push(hand_avail + gy_subs_available);
                }
                CostSource::Mill => caps.push(deck_size),
                CostSource::Graveyard => caps.push(gy_size),
                CostSource::Sacrifice => caps.push(board_creatures),
                CostSource::SelfExile => {}
                CostSource::Attached => {}
            }
        }
        let max_x = caps.into_iter().min().unwrap_or(0).min(10) as i32;
        if max_x < 1 {
            return BuildChoiceResult::UnaffordableX { picked_is_creature };
        }
        let x = oracle.choose_int(
            state,
            ChooseIntRequest {
                min: 1,
                max: max_x,
                prompt: format!("X for {}", short(picked)),
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
            let mut remaining = hand_needed;
            if identity_count < remaining {
                let want_gy = remaining - identity_count;
                let gy_subs = state.find_gy_hand_substitutes(active, picked, want_gy);
                let used = gy_subs.len();
                choices.gy_hand_payment_ids = gy_subs;
                remaining -= used;
            }
            if remaining > 0 {
                choices.hand_payment_ids =
                    state.resolve_hand_payment(active, picked, remaining, oracle);
            }
        }
        let raw_gy_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Graveyard))
            .map(|c| {
                if c.is_x {
                    x.max(0) as usize
                } else {
                    c.amount.max(0) as usize
                }
            })
            .sum();
        let gy_red = state.cost_reduction(picked, CostSource::Graveyard).max(0) as usize;
        let gy_needed = raw_gy_needed.saturating_sub(gy_red);
        if gy_needed > 0 {
            choices.graveyard_payment_ids =
                state.resolve_graveyard_payment(active, picked, gy_needed);
        }
    } else if matches!(kind, CardType::Creature) {
        let raw_hand_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Hand))
            .map(|c| c.amount.max(0) as usize)
            .sum();
        let hand_red = state.cost_reduction(picked, CostSource::Hand).max(0) as usize;
        let mut hand_needed = raw_hand_needed.saturating_sub(hand_red);
        if hand_needed > 0 {
            if let Some(jewel) = state.find_jewel_tap_candidate(active, picked) {
                choices.jewel_tap = Some(jewel);
                hand_needed -= 1;
            }
        }
        if hand_needed > 0 {
            let identity_match_count = state.identity_matching_hand_count(active, picked);
            if identity_match_count < hand_needed {
                let want_gy = hand_needed - identity_match_count;
                let gy_subs = state.find_gy_hand_substitutes(active, picked, want_gy);
                let used = gy_subs.len();
                choices.gy_hand_payment_ids = gy_subs;
                hand_needed -= used;
            }
        }
        if hand_needed > 0 {
            choices.hand_payment_ids =
                state.resolve_hand_payment(active, picked, hand_needed, oracle);
        }
        let raw_gy_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Graveyard))
            .map(|c| c.amount.max(0) as usize)
            .sum();
        let gy_red = state.cost_reduction(picked, CostSource::Graveyard).max(0) as usize;
        let gy_needed = raw_gy_needed.saturating_sub(gy_red);
        if gy_needed > 0 {
            choices.graveyard_payment_ids =
                state.resolve_graveyard_payment(active, picked, gy_needed);
        }
        let has_setup_cost = cost
            .iter()
            .any(|c| matches!(c.source, CostSource::Sacrifice | CostSource::Graveyard));
        if !has_setup_cost && !active_is_human {
            rig_creature_free_haste(state, picked);
        }
    } else if matches!(
        kind,
        CardType::Spell | CardType::Artifact | CardType::Mutation | CardType::Unspecified
    ) {
        let raw_hand_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Hand))
            .map(|c| c.amount.max(0) as usize)
            .sum();
        let hand_red = state.cost_reduction(picked, CostSource::Hand).max(0) as usize;
        let mut hand_needed = raw_hand_needed.saturating_sub(hand_red);
        if hand_needed > 0 {
            if let Some(jewel) = state.find_jewel_tap_candidate(active, picked) {
                choices.jewel_tap = Some(jewel);
                hand_needed -= 1;
            }
        }
        if hand_needed > 0 {
            let identity_match_count = state.identity_matching_hand_count(active, picked);
            if identity_match_count < hand_needed {
                let want_gy = hand_needed - identity_match_count;
                let gy_subs = state.find_gy_hand_substitutes(active, picked, want_gy);
                let used = gy_subs.len();
                choices.gy_hand_payment_ids = gy_subs;
                hand_needed -= used;
            }
        }
        if hand_needed > 0 {
            choices.hand_payment_ids =
                state.resolve_hand_payment(active, picked, hand_needed, oracle);
        }
        let raw_gy_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Graveyard))
            .map(|c| c.amount.max(0) as usize)
            .sum();
        let gy_red = state.cost_reduction(picked, CostSource::Graveyard).max(0) as usize;
        let gy_needed = raw_gy_needed.saturating_sub(gy_red);
        if gy_needed > 0 {
            choices.graveyard_payment_ids =
                state.resolve_graveyard_payment(active, picked, gy_needed);
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

    // Sacrifice slots (any kind): pick lowest-value first.
    let sacrifice_slots: Vec<Option<CardType>> = cost
        .iter()
        .filter(|c| matches!(c.source, CostSource::Sacrifice))
        .flat_map(|c| {
            let n = c.amount.max(0) as usize;
            std::iter::repeat_n(c.kind, n)
        })
        .collect();
    if !sacrifice_slots.is_empty() {
        let mut used: std::collections::BTreeSet<InstanceId> = std::collections::BTreeSet::new();
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
            sac_candidates.sort_by_key(|iid| sacrifice_keep_value(state, iid));
            if let Some(pick) = sac_candidates.into_iter().next() {
                used.insert(pick.clone());
                choices.sacrifice_ids.push(pick);
            }
        }
    }

    // P.31 ATTACHED-source.
    let raw_attached_need: usize = cost
        .iter()
        .filter(|c| matches!(c.source, CostSource::Attached))
        .map(|c| {
            if c.is_x {
                choices.x_value.unwrap_or(0).max(0) as usize
            } else {
                c.amount.max(0) as usize
            }
        })
        .sum();
    let att_red = state.cost_reduction(picked, CostSource::Attached).max(0) as usize;
    let attached_need = raw_attached_need.saturating_sub(att_red);
    if attached_need > 0 {
        let mut pool = state.find_attached_payments(active, usize::MAX);
        pool.sort_by_key(|aid| attached_keep_value(state, aid));
        pool.truncate(attached_need);
        choices.attached_payment_ids = pool;
    }

    BuildChoiceResult::Choices(choices)
}

pub fn run_game_continue(
    state: &mut GameState,
    rng: &mut StdRng,
    log: &mut Vec<String>,
    lua: &mlua::Lua,
    ais: &[super::AiKind; 2],
) -> GameStats {
    let oracle_seed: u64 = rng.gen();
    // If either side is Human, wrap the random oracle so its choose_*
    // calls route to the human when the asker matches.
    let human_pair: Option<(PlayerId, std::sync::Arc<super::human::HumanInterface>)> = ais
        .iter()
        .enumerate()
        .find_map(|(idx, ai)| match ai {
            super::AiKind::Human(iface) => Some((
                if idx == 0 { PlayerId::A } else { PlayerId::B },
                iface.clone(),
            )),
            _ => None,
        });
    let mut oracle = RecordingOracle::new(super::human::HumanAwareOracle::new(
        RandomOracle::new(StdRng::seed_from_u64(oracle_seed)),
        human_pair,
    ));

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
        card_play_turn_events: Vec::new(),
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

    // Per-game wall-clock watchdog. A release game terminates in well
    // under a second on typical hardware; debug ~5x slower. The budget
    // is generous (default 30s) so a slow-but-progressing game isn't
    // killed. When it fires, we dump active player's hand+board+GY card
    // ids + the most recently picked / activated card so the prune /
    // EA harness can identify the culprit. The hung game is scored as
    // a loss for the active player (couldn't make progress on their
    // turn — conservative). Tunable via `TSOT_GAME_TIMEOUT_SECS`.
    let timeout = std::env::var("TSOT_GAME_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30));
    let game_start = Instant::now();
    // Reset per turn so the timeout report identifies the actual offending
    // card, not a stale pick from an earlier successful turn.
    let mut last_picked: Option<InstanceId> = None;
    let mut last_activated: Option<(InstanceId, usize)> = None;
    // Heartbeat: log progress every 5s for games that don't finish
    // promptly. Helps identify slow-but-not-hung games before the
    // wall-clock cap fires.
    let mut last_heartbeat = game_start;

    let mut safety = 1000;
    while state.winner.is_none() && safety > 0 {
        if game_start.elapsed() > timeout {
            report_game_timeout(
                state,
                "outer turn loop",
                last_picked.as_ref(),
                last_activated.as_ref(),
            );
            state.set_winner(Some(state.active_player.opponent()));
            break;
        }
        if last_heartbeat.elapsed() > Duration::from_secs(5) {
            eprintln!(
                "[HEARTBEAT] elapsed={:.1?} turn={} phase={:?} active={:?} \
                 A_board={} B_board={} A_deck={} B_deck={} chain={}",
                game_start.elapsed(),
                state.turn,
                state.phase,
                state.active_player,
                state.a.board.len(),
                state.b.board.len(),
                state.a.deck.len(),
                state.b.deck.len(),
                state.priority.as_ref().map(|p| p.chain.len()).unwrap_or(0),
            );
            last_heartbeat = Instant::now();
        }
        safety -= 1;
        let active = state.active_player;
        let turn = state.turn;
        last_picked = None;
        last_activated = None;
        let mut events: Vec<String> = Vec::new();

        while state.phase != Phase::Main1 && state.winner.is_none() {
            state.next_phase();
        }
        if state.winner.is_some() {
            log.push(format!("turn {turn} ({active:?}): deck-out before Main1"));
            break;
        }

        // Multi-card-per-turn (Pattern B): at most one creature per turn,
        // but as many non-creatures as the AI can afford. Inner safety
        // cap (`pattern_b_iter`) catches "picker keeps returning the
        // same card → `continue` → re-pick" infinite loops (e.g., an
        // X-cost spell that affords pick + is unaffordable at X-pick
        // resolution, in lockstep with another bug). 200 iterations is
        // a generous ceiling — a real turn never plays more than ~10.
        let mut played_creature = false;
        let mut pattern_b_iter: u32 = 0;
        loop {
            pattern_b_iter += 1;
            if pattern_b_iter > 200 {
                report_game_timeout(
                    state,
                    "Pattern B inner loop",
                    last_picked.as_ref(),
                    last_activated.as_ref(),
                );
                state.set_winner(Some(state.active_player.opponent()));
                break;
            }
            if game_start.elapsed() > timeout {
                report_game_timeout(
                    state,
                    "Pattern B inner loop (wall-clock)",
                    last_picked.as_ref(),
                    last_activated.as_ref(),
                );
                state.set_winner(Some(state.active_player.opponent()));
                break;
            }
            if state.winner.is_some() {
                break;
            }
            let kind_filter = if played_creature {
                PickKindFilter::NonCreatureOnly
            } else {
                PickKindFilter::Any
            };
            // Per-player AI: each player can run its own decision policy.
            // Default (Heuristic for both) is byte-identical to pre-MCTS
            // behavior; mixed Heuristic/Mcts is how matchup-mcts compares.
            let pick = match &ais[active.index()] {
                super::AiKind::Heuristic => {
                    pick_random_playable_in_hand(state, active, rng, kind_filter)
                }
                super::AiKind::Mcts(mcts_cfg) => {
                    super::mcts::pick_play(state, active, kind_filter, mcts_cfg, lua)
                }
                super::AiKind::Human(iface) => {
                    let candidates =
                        super::ai::enumerate_playable_in_hand(state, active, kind_filter);
                    let activations = enumerate_human_activations(state, active);
                    let iface = iface.clone();
                    match iface.main_phase_choice(state, active, candidates, kind_filter, activations) {
                        super::human::MainPhaseChoice::Pass => None,
                        super::human::MainPhaseChoice::Play(iid) => Some(iid),
                        super::human::MainPhaseChoice::Activate { iid, ability_index, x } => {
                            last_activated = Some((iid.clone(), ability_index));
                            // Fire the activation. Failures are logged
                            // and the loop continues — the user might
                            // pick another action.
                            if let Err(e) = state.activate_ability(
                                &iid,
                                ability_index,
                                x,
                                Some(&mut EventContext::new(lua, &mut oracle)),
                            ) {
                                log.push(format!(
                                    "turn {turn} ({active:?}): human activation {iid}[{ability_index}] failed: {e:?}"
                                ));
                            }
                            continue;
                        }
                    }
                }
            };
            let Some(picked) = pick else {
                break;
            };
            last_picked = Some(picked.clone());
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
                // Build PlayChoices via the shared choice-builder (same
                // function MCTS rollouts use). Sacrifice-stats bumping
                // stays here (Pattern B's concern, not the builder's).
                let build_result = build_pattern_b_choices(
                    state,
                    active,
                    &picked,
                    &mut oracle,
                    matches!(ais[active.index()], super::AiKind::Human(_)),
                );
                let choices = match build_result {
                    BuildChoiceResult::Choices(c) => c,
                    BuildChoiceResult::UnaffordableX { picked_is_creature: pic } => {
                        if pic {
                            played_creature = true;
                        } else {
                            break;
                        }
                        continue;
                    }
                };
                // Update sacrifice telemetry from the picked ids.
                for sac_iid in &choices.sacrifice_ids {
                    if let Some(card_id) = state.card_pool.get(sac_iid).map(|c| c.card.id.clone()) {
                        *stats.card_sacrificed_count.entry(card_id).or_insert(0) += 1;
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
                // Per-cast wall-clock tripwire: a single cast resolving for >1s
                // suggests a Lua handler in a tight loop (no Rust-side watchdog
                // covers handler-internal time).
                let cast_start = Instant::now();
                let result = state.play_card(
                    active,
                    &picked,
                    choices,
                    Some(&mut EventContext::new(lua, &mut oracle)),
                );
                let cast_elapsed = cast_start.elapsed();
                if cast_elapsed > Duration::from_secs(1) {
                    let card_id = state
                        .card_pool
                        .get(&picked)
                        .map(|c| c.card.id.clone())
                        .unwrap_or_else(|| format!("?{picked}"));
                    eprintln!(
                        "[SLOW CAST] turn={} active={:?} card={} elapsed={:.2?} result={:?}",
                        state.turn, active, card_id, cast_elapsed, result,
                    );
                    state.bump_action("slow_cast", active);
                }
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
                // "Suicide skip": if the engine landed in a state where
                // the active player has already lost (e.g., a Lua handler
                // killed them mid-cast), the AI rolls back so its
                // preview doesn't commit a fatal play. For HUMAN sides
                // we never auto-skip — the player chose this card and
                // is allowed to make any legal play, even a bad one.
                let active_is_human = matches!(ais[active.index()], super::AiKind::Human(_));
                let mut suicide = !active_is_human && state.winner == Some(opponent_of_active);
                let preview_size = state.journal.as_ref().map(|j| j.len()).unwrap_or(0) as u64;

                bump_preview_attempt(&mut stats, active, preview_size);

                let mut result = result;
                if suicide && !response_fired {
                    if let Some(flipped) = ScriptedOracle::flip_first_player(oracle.recording()) {
                        if let Some(journal) = state.journal.take() {
                            journal.rollback(state);
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
                            .entry(card_id.clone())
                            .and_modify(|(min_t, max_t)| {
                                if turn_now < *min_t {
                                    *min_t = turn_now;
                                }
                                if turn_now > *max_t {
                                    *max_t = turn_now;
                                }
                            })
                            .or_insert((turn_now, turn_now));
                        // Full distribution (vs the (min, max) summary
                        // above) — feeds the turn-curve aggregation in
                        // `tsot curve-sample` → `cards-report.lua`.
                        // Player kept so a future per-deck analysis
                        // can group; today's consumer ignores it.
                        stats
                            .card_play_turn_events
                            .push((card_id, turn_now, active));
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
                        journal.rollback(state);
                    }
                    bump_preview_rollback(&mut stats, active);
                    if suicide {
                        state.bump_action("preview_skip_suicide", active);
                    }
                    // Surface the failure reason so the UI / log isn't
                    // blind to silent rollbacks. AI rollbacks happen
                    // often (preview-and-skip is the design); human-side
                    // rollbacks should be rare and worth flagging.
                    if let Err(err) = result {
                        let card_id = state
                            .card_pool
                            .get(&picked)
                            .map(|c| c.card.id.clone())
                            .unwrap_or_else(|| picked.clone());
                        log.push(format!(
                            "turn {turn} ({active:?}): play_card({card_id}) failed: {err:?}{}",
                            if active_is_human { " [HUMAN — visible failure]" } else { "" }
                        ));
                    } else if suicide {
                        let card_id = state
                            .card_pool
                            .get(&picked)
                            .map(|c| c.card.id.clone())
                            .unwrap_or_else(|| picked.clone());
                        log.push(format!(
                            "turn {turn} ({active:?}): {card_id} rolled back (would have lost the game)"
                        ));
                    }
                    if picked_is_creature {
                        played_creature = true;
                    } else {
                        break;
                    }
                }
            }
        }

        // Pre-combat activation pass: fire activated abilities on
        // non-creature board cards (artifacts, mostly jewels). Drawing
        // before combat lets the AI know its hand for the rest of the
        // turn. Creatures hold their activations until post-combat so
        // tapping for an ability doesn't pre-empt an attack.
        let pre_acts =
            run_activation_pass(state, active, lua, &mut oracle, true, &mut last_activated, ais);
        if pre_acts > 0 {
            events.push(format!("{pre_acts} pre-combat activation(s)"));
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
        let attackers: Vec<InstanceId> = match &ais[active.index()] {
            super::AiKind::Heuristic | super::AiKind::Mcts(_) => select_attackers(state, active),
            super::AiKind::Human(iface) => {
                let eligible = super::ai::eligible_attackers(state, active);
                iface.pick_attackers(state, active, eligible)
            }
        };
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
            let assignments = match &ais[defender.index()] {
                super::AiKind::Heuristic | super::AiKind::Mcts(_) => pick_blocks(state, defender),
                super::AiKind::Human(iface) => {
                    use tsot::game::CombatState;
                    let declared: Vec<InstanceId> = match &state.combat {
                        Some(CombatState::AwaitingBlockers { attacks }) => {
                            attacks.iter().map(|a| a.attacker.clone()).collect()
                        }
                        _ => Vec::new(),
                    };
                    let eligible = super::ai::eligible_blockers(state, defender);
                    iface.pick_blocks(state, defender, declared, eligible)
                }
            };
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

        // Post-combat activation pass: anything still untapped that can
        // activate, fires now. Vigilance creatures that swung this turn
        // are still untapped here — this is where vigilant-human draws.
        // Pre-combat non-creature activations already tapped those, so
        // they're naturally excluded by `can_activate`.
        let post_acts =
            run_activation_pass(state, active, lua, &mut oracle, false, &mut last_activated, ais);
        if post_acts > 0 {
            events.push(format!("{post_acts} post-combat activation(s)"));
        }

        // Human-side Main2 prompt loop. Engine has a Main2 phase but
        // the turn-progression code skips through it without a prompt
        // — fine for AI (which auto-activated post-combat), wrong for
        // human. Run a Pattern-B-style loop here: plays + activations
        // until the human passes.
        if let super::AiKind::Human(iface) = &ais[active.index()] {
            let iface = iface.clone();
            // Advance into Main2 explicitly so play_card timing checks
            // (sorcery-speed) accept the cast.
            while state.phase != Phase::Main2 && state.winner.is_none() {
                state.next_phase();
                if matches!(state.phase, Phase::Untap | Phase::Draw) {
                    // We've already wrapped past End — bail.
                    break;
                }
            }
            let mut m2_played_creature = false;
            loop {
                if state.winner.is_some() {
                    break;
                }
                let kind_filter = if m2_played_creature {
                    PickKindFilter::NonCreatureOnly
                } else {
                    PickKindFilter::Any
                };
                let candidates =
                    super::ai::enumerate_playable_in_hand(state, active, kind_filter);
                let activations = enumerate_human_activations(state, active);
                if candidates.is_empty() && activations.is_empty() {
                    break;
                }
                match iface.main_phase_choice(
                    state,
                    active,
                    candidates,
                    kind_filter,
                    activations,
                ) {
                    super::human::MainPhaseChoice::Pass => break,
                    super::human::MainPhaseChoice::Activate { iid, ability_index, x } => {
                        last_activated = Some((iid.clone(), ability_index));
                        if let Err(e) = state.activate_ability(
                            &iid,
                            ability_index,
                            x,
                            Some(&mut EventContext::new(lua, &mut oracle)),
                        ) {
                            log.push(format!(
                                "turn {turn} ({active:?}): main2 activation {iid}[{ability_index}] failed: {e:?}"
                            ));
                        }
                    }
                    super::human::MainPhaseChoice::Play(picked) => {
                        let picked_is_creature = state
                            .card_pool
                            .get(&picked)
                            .map(|c| c.card.kind == CardType::Creature)
                            .unwrap_or(false);
                        let build_result = build_pattern_b_choices(
                            state, active, &picked, &mut oracle, true,
                        );
                        let choices = match build_result {
                            BuildChoiceResult::Choices(c) => c,
                            BuildChoiceResult::UnaffordableX { .. } => continue,
                        };
                        oracle.clear();
                        let result = state.play_card(
                            active,
                            &picked,
                            choices,
                            Some(&mut EventContext::new(lua, &mut oracle)),
                        );
                        if let Err(err) = result {
                            let card_id = state
                                .card_pool
                                .get(&picked)
                                .map(|c| c.card.id.clone())
                                .unwrap_or_else(|| picked.clone());
                            log.push(format!(
                                "turn {turn} ({active:?}): main2 play_card({card_id}) failed: {err:?}"
                            ));
                        } else if picked_is_creature {
                            m2_played_creature = true;
                        }
                    }
                }
            }
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
    // `replay_journal` lifecycle is the caller's responsibility — the
    // wrapper [`run_game`] takes + sets `stats.replay_journal_entries`;
    // MCTS rollouts take and rollback instead. We just return stats.
    stats
}

/// Build the activatable-abilities list a human main-phase prompt
/// surfaces. Walks the player's board, expands each card's
/// activations, and includes any whose `can_activate` check passes
/// right now (the same predicate the heuristic activation pass uses).
fn enumerate_human_activations(
    state: &GameState,
    player: PlayerId,
) -> Vec<super::human::ActivationOption> {
    let mut out = Vec::new();
    let ids: Vec<InstanceId> = state.player(player).board.clone();
    for iid in &ids {
        let n = state.activation_count(iid);
        if n == 0 {
            continue;
        }
        let card_name = state
            .card_pool
            .get(iid)
            .map(|i| i.card.name.clone())
            .unwrap_or_else(|| iid.clone());
        for idx in 0..n {
            if !state.can_activate(iid, idx) {
                continue;
            }
            let (text, needs_x) = state
                .activation_at(iid, idx)
                .map(|a| (a.text.clone(), a.cost_components.iter().any(|c| c.is_x)))
                .unwrap_or_else(|| (format!("ability {idx}"), false));
            out.push(super::human::ActivationOption {
                iid: iid.clone(),
                card_name: card_name.clone(),
                ability_index: idx,
                text,
                needs_x,
            });
        }
    }
    out
}

/// Fire activated abilities the player can currently afford. Walks
/// their board, considers each card's first activatable ability, and
/// activates it if eligible. `non_creatures_only = true` restricts the
/// pass to non-creature cards (used pre-combat, so creatures stay free
/// for attack decisions). Returns the number of activations fired.
fn run_activation_pass(
    state: &mut GameState,
    player: PlayerId,
    lua: &mlua::Lua,
    oracle: &mut dyn ChoiceOracle,
    non_creatures_only: bool,
    last_activated: &mut Option<(InstanceId, usize)>,
    ais: &[super::AiKind; 2],
) -> u32 {
    // Activated abilities require the controller's opt-in. AI sides
    // keep auto-firing (preserves heuristic behavior + test
    // determinism). Human side never enters this pass — `Activate`
    // is a main-phase action the player drives explicitly.
    let active_is_human = matches!(ais[player.index()], super::AiKind::Human(_));
    if active_is_human {
        return 0;
    }
    let mut count = 0u32;
    // Snapshot board ids up front. Activation handlers can mutate the
    // board (move cards in/out), so we re-validate membership and
    // re-fetch instance data on each iteration.
    let ids: Vec<InstanceId> = state.player(player).board.clone();
    for iid in &ids {
        let is_creature = match state.card_pool.get(iid) {
            Some(inst) => inst.card.kind == CardType::Creature,
            None => continue,
        };
        // RULES A.5+: total activations = printed (`card.activated`)
        // plus static-granted (`StaticDef.granted_activated`).
        let abilities_n = state.activation_count(iid);
        if abilities_n == 0 {
            continue;
        }
        if non_creatures_only && is_creature {
            continue;
        }
        // Activate the first eligible ability. Tap is consumed by the
        // first activation so additional abilities on the same card
        // won't pass `can_activate` until next untap.
        for idx in 0..abilities_n {
            if !state.can_activate(iid, idx) {
                continue;
            }
            // AI sides keep the auto-fire behavior. Human side never
            // auto-fires — the human drives activations explicitly via
            // the main-phase `Activate` action. Skip the entire loop
            // for human (we exit via `return 0` below).
            let _ = active_is_human;
            // For X-cost activations, the AI commits to a concrete X
            // before calling. Simple heuristic: spend ~half the hand
            // (rounded up) so we keep some cards in hand. Bigger X
            // when hand is large; minimum X=1.
            let needs_x = state
                .activation_at(iid, idx)
                .map(|a| a.cost_components.iter().any(|c| c.is_x))
                .unwrap_or(false);
            let x_value = if needs_x {
                let hand_size = state.player(player).hand.len() as i32;
                // Heuristic: spend up to half the hand, capped at 5,
                // minimum 1. Refined later — this is a v1 stake.
                Some(((hand_size + 1) / 2).clamp(1, 5))
            } else {
                None
            };
            *last_activated = Some((iid.clone(), idx));
            if state
                .activate_ability(iid, idx, x_value, Some(&mut EventContext::new(lua, oracle)))
                .is_ok()
            {
                count += 1;
                break;
            }
        }
    }
    count
}

/// Dump game state to stderr when the wall-clock watchdog or inner-loop
/// safety cap trips. Reports active player, turn, zone contents (by
/// `card.id`, not InstanceId — readable correlation to `cards/*.lua`),
/// most recent card picked, and most recent activation. Operator greps
/// stderr for `[GAME TIMEOUT]` to identify the suspect card.
fn report_game_timeout(
    state: &GameState,
    site: &str,
    last_picked: Option<&InstanceId>,
    last_activated: Option<&(InstanceId, usize)>,
) {
    tsot::game::bump_timeout_and_maybe_halt(site);
    let ids = |iids: &[InstanceId]| -> Vec<String> {
        iids.iter()
            .filter_map(|i| state.card_pool.get(i).map(|c| c.card.id.clone()))
            .collect()
    };
    let card_id_of = |iid: &InstanceId| -> String {
        state
            .card_pool
            .get(iid)
            .map(|c| c.card.id.clone())
            .unwrap_or_else(|| format!("?{iid}"))
    };
    eprintln!(
        "[GAME TIMEOUT] site={site} turn={} active={:?} winner={:?}",
        state.turn, state.active_player, state.winner,
    );
    if let Some(p) = last_picked {
        eprintln!("  last_picked: {} ({})", p, card_id_of(p));
    }
    if let Some((iid, idx)) = last_activated {
        eprintln!("  last_activated: {} ({}) ability_idx={idx}", iid, card_id_of(iid));
    }
    eprintln!("  A hand: {:?}", ids(&state.a.hand));
    eprintln!("  A board: {:?}", ids(&state.a.board));
    eprintln!("  A graveyard: {:?}", ids(&state.a.graveyard));
    eprintln!("  B hand: {:?}", ids(&state.b.hand));
    eprintln!("  B board: {:?}", ids(&state.b.board));
    eprintln!("  B graveyard: {:?}", ids(&state.b.graveyard));
    eprintln!(
        "  decks: A={} B={} | priority_chain={}",
        state.a.deck.len(),
        state.b.deck.len(),
        state.priority.as_ref().map(|p| p.chain.len()).unwrap_or(0),
    );
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
    fn sim_pays_attached_cost_with_real_baseline_decks() {
        // Play N games using the project's curated baseline decks
        // against themselves. These are EA-evolved/tournament-vetted
        // 50-card decks with realistic cost variety. Assert at least
        // one P.31 attached-payment fires across the matchup.
        use crate::sim::evolved_deck::EvolvedDeck;
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let baseline_dir = std::path::Path::new("baselines");
        let mut decks: Vec<Vec<tsot::card::Card>> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(baseline_dir).unwrap().flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match EvolvedDeck::load(&p) {
                Ok(saved) => match saved.to_cards(&registry) {
                    Ok(cards) => {
                        labels.push(p.file_name().unwrap().to_string_lossy().into_owned());
                        decks.push(cards);
                    }
                    Err(e) => eprintln!("baseline {} to_cards err: {e}", p.display()),
                },
                Err(e) => eprintln!("baseline {} load err: {e}", p.display()),
            }
        }
        assert!(!decks.is_empty(), "expected at least one baseline deck");
        eprintln!("loaded baselines: {:?}", labels);
        // Play each baseline against each other (including mirror) for
        // a few seeds. One mirror match per deck minimum.
        let mut total_transfer: u32 = 0;
        let mut total_exile: u32 = 0;
        for (i, deck_a) in decks.iter().enumerate() {
            for (j, deck_b) in decks.iter().enumerate() {
                let state = GameState::new(deck_a.clone(), deck_b.clone());
                let seed = 0xBA5E0000_u64 + (i as u64) * 100 + (j as u64);
                let mut rng = StdRng::seed_from_u64(seed);
                let mut log: Vec<String> = Vec::new();
                let (stats, _journal) =
                    run_game(state, &mut rng, &mut log, registry.lua());
                total_transfer += stats
                    .action_counts
                    .get("attached_payment_transfer")
                    .map(|v| v[0] + v[1])
                    .unwrap_or(0);
                total_exile += stats
                    .action_counts
                    .get("attached_payment_exile")
                    .map(|v| v[0] + v[1])
                    .unwrap_or(0);
            }
        }
        eprintln!(
            "P.31 baseline matchups: {} games, transfer={} exile={}",
            decks.len() * decks.len(),
            total_transfer,
            total_exile,
        );
        assert!(
            total_transfer + total_exile > 0,
            "expected ≥1 P.31 attached-payment across {} baseline matchups. transfer={total_transfer} exile={total_exile}",
            decks.len() * decks.len()
        );
    }

    /// Strongest journal-rollback invariant: open the replay journal at
    /// game start, run a FULL random game to completion via
    /// `run_game_continue`, then rollback — final state must equal
    /// initial state byte-for-byte.
    ///
    /// `tests/journal_full_game_rollback.rs` exercises the scripted
    /// 3-turn version using only the public engine API. This in-crate
    /// test uses `run_game_continue` directly to cover the full
    /// Pattern B / X-cost / response window flow that MCTS rollouts
    /// actually trip into.
    ///
    /// If this test fails, some mutation site is not journaled. MCTS
    /// rollout safety depends on this rollback being byte-identical;
    /// the test diff identifies which field (and thus mutation site)
    /// is the gap.
    #[test]
    fn full_random_game_rollback_restores_initial_state() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        // Pick a vanilla creature template so the deck contains
        // handler-free cards — the test exercises journaling for the
        // hot-path Pattern B / combat / turn machinery, not for one
        // specific card's Lua side effects.
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, tsot::card::CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.len() == 1
                    && !c.cost[0].is_x
            })
            .expect("a vanilla creature should exist in cards/")
            .clone();
        let deck_a: Vec<tsot::card::Card> = (0..50).map(|_| template.clone()).collect();
        let deck_b: Vec<tsot::card::Card> = (0..50).map(|_| template.clone()).collect();

        let mut state = GameState::new(deck_a, deck_b);

        // Clone the initial state for per-field comparison after rollback.
        // Cloning a 100-card pool is expensive but acceptable for a
        // one-shot diagnostic test.
        let initial = state.clone();

        state.replay_journal = Some(tsot::game::Journal::new());

        let mut rng = StdRng::seed_from_u64(0xC0FFEE);
        let mut log: Vec<String> = Vec::new();
        let ais = [super::super::AiKind::Heuristic, super::super::AiKind::Heuristic];
        let stats = run_game_continue(&mut state, &mut rng, &mut log, registry.lua(), &ais);

        assert!(state.winner.is_some(), "game should have a winner");
        assert!(stats.turns > 0, "stats should record turns");

        let journal = state
            .replay_journal
            .take()
            .expect("replay_journal still open");
        let entry_count = journal.len();
        assert!(entry_count > 0, "a full game should produce many journal entries");
        journal.rollback(&mut state);

        // Compare each top-level field; on a card_pool mismatch, walk
        // per-instance and emit the first divergent CardInstance field.
        assert_eq!(format!("{:?}", initial.active_player), format!("{:?}", state.active_player), "active_player not rolled back");
        assert_eq!(initial.turn, state.turn, "turn not rolled back");
        assert_eq!(format!("{:?}", initial.phase), format!("{:?}", state.phase), "phase not rolled back");
        assert_eq!(format!("{:?}", initial.winner), format!("{:?}", state.winner), "winner not rolled back");
        assert_eq!(format!("{:?}", initial.combat), format!("{:?}", state.combat), "combat not rolled back");
        assert_eq!(format!("{:?}", initial.event_fires), format!("{:?}", state.event_fires), "event_fires not rolled back");
        assert_eq!(format!("{:?}", initial.action_counts), format!("{:?}", state.action_counts), "action_counts not rolled back");
        assert_eq!(format!("{:?}", initial.priority), format!("{:?}", state.priority), "priority not rolled back");
        assert_eq!(format!("{:?}", initial.a), format!("{:?}", state.a), "player A's zones not rolled back");
        assert_eq!(format!("{:?}", initial.b), format!("{:?}", state.b), "player B's zones not rolled back");

        // card_pool: narrow to which CardInstance + which field.
        for (iid, post_inst) in &state.card_pool {
            let init_inst = initial.card_pool.get(iid)
                .unwrap_or_else(|| panic!("instance {iid} appeared post-rollback (not in initial pool)"));
            assert_eq!(init_inst.tapped, post_inst.tapped, "{iid}.tapped not rolled back");
            assert_eq!(init_inst.damage, post_inst.damage, "{iid}.damage not rolled back");
            assert_eq!(init_inst.face_down, post_inst.face_down, "{iid}.face_down not rolled back");
            assert_eq!(init_inst.summoning_sick, post_inst.summoning_sick, "{iid}.summoning_sick not rolled back");
            assert_eq!(init_inst.attacked_this_turn, post_inst.attacked_this_turn, "{iid}.attacked_this_turn not rolled back");
            assert_eq!(format!("{:?}", init_inst.controller), format!("{:?}", post_inst.controller), "{iid}.controller not rolled back");
            assert_eq!(format!("{:?}", init_inst.attached), format!("{:?}", post_inst.attached), "{iid}.attached not rolled back");
            assert_eq!(format!("{:?}", init_inst.modifiers), format!("{:?}", post_inst.modifiers), "{iid}.modifiers not rolled back");
            assert_eq!(format!("{:?}", init_inst.status_effects), format!("{:?}", post_inst.status_effects), "{iid}.status_effects not rolled back");
        }
        for iid in initial.card_pool.keys() {
            assert!(state.card_pool.contains_key(iid), "instance {iid} removed but rollback didn't restore it");
        }
        // Per-instance whole-Debug compare to surface the divergent one.
        for (iid, post_inst) in &state.card_pool {
            let init_inst = initial.card_pool.get(iid).unwrap();
            let init_dbg = format!("{:?}", init_inst);
            let post_dbg = format!("{:?}", post_inst);
            if init_dbg != post_dbg {
                panic!(
                    "instance {iid} differs after rollback. \n\
                     INITIAL: {init_dbg}\n\
                     POST:    {post_dbg}"
                );
            }
        }
        assert_eq!(format!("{:?}", initial.card_pool), format!("{:?}", state.card_pool), "card_pool differs in some field not covered by per-field checks ({entry_count} journal entries)");
    }

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
