//! Sim AI heuristics. Pure-state functions plus the picker used by the
//! `run_game` loop. No mutation of GameStats — all writes happen in
//! [`super::run`].

use rand::seq::SliceRandom;
use rand::Rng;
use tsot::card::{CardType, CostSource};
use tsot::game::{GameState, InstanceId, PlayerId};

/// Filter for which kinds the picker is allowed to return. Used by the
/// multi-card-per-turn loop in run_game (Pattern B caps at one creature
/// per turn but allows multiple non-creatures).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // CreatureOnly currently unused under Pattern B but kept for future per-kind filtering.
pub enum PickKindFilter {
    Any,
    CreatureOnly,
    NonCreatureOnly,
}

pub fn pick_random_playable_in_hand(
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
                CardType::Creature => {
                    let has_setup = inst.card.cost.iter().any(|c| {
                        matches!(c.source, CostSource::Sacrifice | CostSource::Graveyard)
                    });
                    !has_setup || can_pay_instant_cost(state, player, iid)
                }
                CardType::Spell => can_pay_instant_cost(state, player, iid),
                CardType::Artifact => can_pay_instant_cost(state, player, iid),
                CardType::Mutation => {
                    if !can_pay_instant_cost(state, player, iid) {
                        return false;
                    }
                    state.a.board.iter().chain(state.b.board.iter()).any(|t| {
                        state
                            .card_pool
                            .get(t)
                            .map(|i| i.card.kind == CardType::Creature)
                            .unwrap_or(false)
                    })
                }
                _ => false,
            }
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    // Priority-tiered pick: score each candidate once, find the max,
    // then filter to that tier. (Earlier version computed the score
    // twice per candidate.)
    let scored: Vec<(&InstanceId, i32)> = candidates
        .iter()
        .map(|iid| (*iid, play_priority_score(state, iid)))
        .collect();
    let max_priority = scored.iter().map(|(_, s)| *s).max().unwrap_or(0);
    let top: Vec<&InstanceId> = scored
        .into_iter()
        .filter_map(|(iid, s)| if s == max_priority { Some(iid) } else { None })
        .collect();
    top.choose(rng).map(|iid| (*iid).clone())
}

/// Heuristic: how urgent is this card to play THIS TURN? Higher = play
/// sooner. Cards with on-board statics that compound over many turns
/// (cost reductions, anthems, restrictions) should land early.
pub fn play_priority_score(state: &GameState, iid: &InstanceId) -> i32 {
    let Some(inst) = state.card_pool.get(iid) else {
        return 0;
    };
    let mut s = 0i32;
    if let Some(def) = &inst.card.static_def {
        if !def.cost_modifiers.is_empty() {
            s += 50;
        }
        let stat_active = !matches!(def.modifier_x, tsot::ModifierValue::Fixed(0))
            || !matches!(def.modifier_y, tsot::ModifierValue::Fixed(0));
        if stat_active || def.modifier_keyword.is_some() {
            s += 20;
        }
        if !def.restrictions.is_empty() {
            s += 15;
        }
    }
    s
}

pub fn can_pay_instant_cost(state: &GameState, player: PlayerId, iid: &InstanceId) -> bool {
    let Some(inst) = state.card_pool.get(iid) else {
        return false;
    };
    let mut hand_need = 0usize;
    let mut mill_need = 0usize;
    let mut gy_need = 0usize;
    let mut sac_slots: Vec<Option<CardType>> = Vec::new();
    for c in &inst.card.cost {
        if c.is_x {
            return false;
        }
        let amount = c.amount.max(0) as usize;
        match c.source {
            CostSource::Hand => hand_need += amount,
            CostSource::Mill => mill_need += amount,
            CostSource::Graveyard => gy_need += amount,
            CostSource::Sacrifice => {
                for _ in 0..amount {
                    sac_slots.push(c.kind);
                }
            }
            _ => return false,
        }
    }
    let hand_red = state.cost_reduction(iid, CostSource::Hand).max(0) as usize;
    let mill_red = state.cost_reduction(iid, CostSource::Mill).max(0) as usize;
    let gy_red = state.cost_reduction(iid, CostSource::Graveyard).max(0) as usize;
    hand_need = hand_need.saturating_sub(hand_red);
    mill_need = mill_need.saturating_sub(mill_red);
    gy_need = gy_need.saturating_sub(gy_red);
    let p = state.player(player);
    // Identity-match: only hand cards sharing ≥1 element of the casting
    // card's identity set (colors ∪ symbol) count toward hand_have.
    // Colorless+no-symbol casts are wildcards; colorless+no-symbol
    // discards are NOT.
    let cast_ident = state.card_identity(iid);
    let hand_have = if hand_need == 0 || cast_ident.is_empty() {
        p.hand.len().saturating_sub(1)
    } else {
        p.hand
            .iter()
            .filter(|h| *h != iid)
            .filter(|h| {
                let pay_ident = state.card_identity(h);
                !cast_ident.is_disjoint(&pay_ident)
            })
            .count()
    };
    let mut available: Vec<InstanceId> = p.board.clone();
    let mut sac_ok = true;
    for required_kind in &sac_slots {
        let pos = available.iter().position(|iid| {
            if let Some(k) = required_kind {
                state
                    .card_pool
                    .get(iid)
                    .map(|i| i.card.kind == *k)
                    .unwrap_or(false)
            } else {
                true
            }
        });
        match pos {
            Some(idx) => {
                available.remove(idx);
            }
            None => {
                sac_ok = false;
                break;
            }
        }
    }
    hand_have >= hand_need
        && p.deck.len() >= mill_need
        && p.graveyard.len() >= gy_need
        && sac_ok
}

/// Sim heuristic: how valuable would it be to KEEP this on-board card?
/// Higher = more valuable = less preferred for sacrifice. Used by the
/// sacrifice picker AND by the block policy (trade-up).
pub fn sacrifice_keep_value(state: &GameState, iid: &InstanceId) -> i32 {
    let Some(inst) = state.card_pool.get(iid) else {
        return 0;
    };
    let (x, y) = state.effective_stats(iid);
    let cost_weight: i32 = inst.card.cost.iter().map(|c| c.amount.max(0)).sum();
    let attached_count = inst.attached.len() as i32;
    x + y + cost_weight * 2 + attached_count * 2
}

/// Sim heuristic: skip an attack iff the defender has at least one legal
/// blocker AND no legal blocker dies to this attacker's effective X.
pub fn is_attack_worth_declaring(
    state: &GameState,
    attacker: &InstanceId,
    defender: PlayerId,
) -> bool {
    if !state.card_pool.contains_key(attacker) {
        return false;
    }
    if state.has_keyword(attacker, "unblockable") {
        return true;
    }
    let atk_x = state.effective_stats(attacker).0;
    let atk_flying = state.has_keyword(attacker, "flying");

    let mut any_legal_blocker = false;
    let mut any_kill_possible = false;
    for blk_iid in &state.player(defender).board {
        let Some(blk_inst) = state.card_pool.get(blk_iid) else {
            continue;
        };
        if blk_inst.tapped {
            continue;
        }
        if atk_flying && !state.has_keyword(blk_iid, "flying") {
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

pub fn eligible_attackers(state: &GameState, player: PlayerId) -> Vec<InstanceId> {
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
            if state.has_keyword(iid, "defender") {
                return false;
            }
            if inst.summoning_sick && !state.has_keyword(iid, "haste") {
                return false;
            }
            if state.has_restriction(iid, tsot::card::Restriction::CannotAttack) {
                return false;
            }
            true
        })
        .cloned()
        .collect()
}

pub fn eligible_blockers(state: &GameState, player: PlayerId) -> Vec<InstanceId> {
    state
        .player(player)
        .board
        .iter()
        .filter(|iid| {
            let Some(inst) = state.card_pool.get(*iid) else {
                return false;
            };
            !inst.tapped && !state.has_keyword(iid, "cannot-block")
        })
        .cloned()
        .collect()
}

/// Tiered block policy: T3 clean kill (always take), T2 kill-trade with
/// trade-up signal, T4 multi-block (dying only), T1 chump (dying only).
pub fn pick_blocks(state: &GameState, defender: PlayerId) -> Vec<(InstanceId, InstanceId)> {
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

    let total_incoming: i32 = declared
        .iter()
        .map(|a| state.effective_stats(a).0.max(0))
        .sum();
    let deck = state.player(defender).deck.len() as i32;
    let dying = total_incoming >= deck;

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

        // T3: clean kill — blocker survives.
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

        // T2: kill-trade with trade-up.
        let kill_trade = avail
            .iter()
            .filter(|(_, bx, _)| *bx >= *atk_y)
            .min_by_key(|(_, bx, _)| *bx)
            .cloned();
        if let Some((blk, _, _)) = kill_trade {
            let trade_up =
                sacrifice_keep_value(state, atk) > sacrifice_keep_value(state, &blk) + 4;
            if dying || *atk_x >= 2 || trade_up {
                assignments.push((blk.clone(), atk.clone()));
                used.insert(blk);
                remaining_incoming -= atk_x;
                continue;
            }
        }

        // T4: multi-block (dying only).
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

        // T1: chump only if still dying.
        if remaining_incoming >= deck {
            let chump = avail.iter().min_by_key(|(_, bx, _)| *bx).cloned();
            if let Some((blk, _, _)) = chump {
                assignments.push((blk.clone(), atk.clone()));
                used.insert(blk);
                remaining_incoming -= atk_x;
                continue;
            }
        }
    }

    assignments
}

/// Rig a creature to free + haste before the sim plays it. Used for the
/// vast majority of creatures (those without SETUP costs). Lets the sim
/// keep throughput high without exhausting hand resources every turn.
pub fn rig_creature_free_haste(state: &mut GameState, iid: &InstanceId) {
    let inst = state.card_pool.get_mut(iid).unwrap();
    inst.card.cost = vec![];
    inst.card.abilities.push("haste".to_string());
}
