//! Sim AI heuristics. Pure-state functions plus the picker used by the
//! `run_game` loop. No mutation of GameStats — all writes happen in
//! [`super::run`].

use rand::seq::SliceRandom;
use rand::Rng;
use crate::card::{CardType, CostSource};
use crate::game::{GameState, InstanceId, PlayerId};

/// Filter for which kinds the picker is allowed to return. Used by the
/// multi-card-per-turn loop in run_game (Pattern B caps at one creature
/// per turn but allows multiple non-creatures).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)] // CreatureOnly currently unused under Pattern B but kept for future per-kind filtering.
pub enum PickKindFilter {
    Any,
    CreatureOnly,
    NonCreatureOnly,
}

/// Return every playable card in `player`'s hand that passes the
/// `kind_filter`. Same filter as `pick_heuristic_playable_in_hand` uses,
/// just collected instead of randomly chosen. Used by `sim::mcts` for
/// candidate enumeration.
pub fn enumerate_playable_in_hand(
    state: &GameState,
    player: PlayerId,
    kind_filter: PickKindFilter,
) -> Vec<InstanceId> {
    state
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
                    // Always affordability-gate creatures now that the
                    // `rig_creature_free_haste` shortcut is gone — plain
                    // 1-hand creatures used to be unconditionally
                    // playable (the rig wiped their cost mid-cast), but
                    // they need a real check now: hand of size 1 can't
                    // pay a 1-hand cost (the card itself leaves hand at
                    // cast announcement per P.33 and there's nothing
                    // left to discard). Without this, the picker keeps
                    // re-returning an unplayable creature and Pattern B
                    // loops on the same iid forever.
                    can_pay_instant_cost(state, player, iid)
                }
                CardType::Spell => can_pay_instant_cost(state, player, iid),
                CardType::Artifact => can_pay_instant_cost(state, player, iid),
                CardType::Mutation => {
                    if !can_pay_instant_cost(state, player, iid) {
                        return false;
                    }
                    // Use the shared eligibility helper so the picker
                    // can't offer a mutation whose only viable targets
                    // get refused at play_card (e.g., glass-insect
                    // CannotBeAttachedTo or C.14 transparent mismatch).
                    !state.eligible_mutation_targets(iid).is_empty()
                }
                // Typeless casts (P.1 default to GRAVEYARD; SelfExile
                // shortcut to EXILE). Affordability-gated like spells —
                // SELF is trivially payable per `can_pay_instant_cost`,
                // so the Clear cycle gets picked here.
                CardType::Unspecified => can_pay_instant_cost(state, player, iid),
                _ => false,
            }
        })
        .cloned()
        .collect()
}

/// Collapse a candidate list down to one representative per
/// `card.id`. Multiple copies of the same card in hand produce
/// identical successor states when played; exploring each separately
/// burns search budget on redundant branches. Used by every AI
/// picker (heuristic, UCT, MCTS) before scoring or rolling out.
///
/// First-occurrence wins. Ordering of distinct ids is preserved.
pub fn dedup_candidates_by_card_id(
    state: &GameState,
    candidates: Vec<InstanceId>,
) -> Vec<InstanceId> {
    use std::collections::BTreeSet;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<InstanceId> = Vec::with_capacity(candidates.len());
    for iid in candidates {
        let id = state
            .card_pool
            .get(&iid)
            .map(|i| i.card.id.clone())
            .unwrap_or_default();
        if seen.insert(id) {
            out.push(iid);
        }
    }
    out
}

pub fn pick_heuristic_playable_in_hand(
    state: &GameState,
    player: PlayerId,
    rng: &mut impl Rng,
    kind_filter: PickKindFilter,
) -> Option<InstanceId> {
    // O6: bracket the whole pick decision with `Instant::now()` so
    // the emitted AiPick event carries duration_us. Cheap no-op
    // when trace is off.
    let trace_active = crate::trace::is_enabled();
    let t0 = trace_active.then(std::time::Instant::now);

    // Dedup: copies of the same card.id collapse into one
    // representative. With 6 blue-monkeys, picking among 6 iids is
    // burned search budget — same successor state every time.
    let candidates = dedup_candidates_by_card_id(
        state,
        enumerate_playable_in_hand(state, player, kind_filter),
    );
    // Score each candidate once (used both for the pick and for the
    // emitted AiPick record).
    let scored: Vec<(&InstanceId, i32)> = candidates
        .iter()
        .map(|iid| (iid, play_priority_score(state, iid)))
        .collect();
    let max_priority = scored.iter().map(|(_, s)| *s).max().unwrap_or(0);
    let chosen = if scored.is_empty() {
        None
    } else {
        let top: Vec<&InstanceId> = scored
            .iter()
            .filter_map(|(iid, s)| if *s == max_priority { Some(*iid) } else { None })
            .collect();
        top.choose(rng).map(|iid| (*iid).clone())
    };

    if let Some(t0) = t0 {
        let trace_candidates: Vec<crate::trace::CandidateScore> = scored
            .iter()
            .map(|(iid, score)| crate::trace::CandidateScore {
                iid: (*iid).clone(),
                score: *score,
                rejected_reason: None,
            })
            .collect();
        crate::trace::push(crate::trace::TraceEvent::AiPick {
            at_us: crate::trace::now_us(),
            ai: "Heuristic".to_string(),
            candidates: trace_candidates,
            chosen: chosen.clone(),
            duration_us: t0.elapsed().as_micros() as u64,
        });
    }
    chosen
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
        let stat_active = !matches!(def.modifier_x, crate::ModifierValue::Fixed(0))
            || !matches!(def.modifier_y, crate::ModifierValue::Fixed(0));
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
    // RULES P.32: refuse if the card declares a target category and no
    // legal target exists. Mirrors the engine's cast-time gate so the
    // picker doesn't burn rolls on cards play_card will refuse.
    if let Some(target) = inst.card.target {
        if !state.is_target_legal(target) {
            return false;
        }
    }
    let mut hand_need = 0usize;
    let mut mill_need = 0usize;
    let mut gy_need = 0usize;
    let mut attached_need = 0usize;
    let mut sac_slots: Vec<Option<CardType>> = Vec::new();
    // Variable-X handling: an is_x component contributes X * (component
    // amount, typically 1) to its source's need. The AI doesn't pick X
    // here — that happens in the play loop via oracle.choose_int. For
    // affordability, treat is_x as needing 1 of the resource minimum:
    // the cast is "useful" iff at least X=1 is payable. X=0 makes the
    // cast a no-op, so we don't bother accepting cards we'd cast for X=0.
    for c in &inst.card.cost {
        let amount = if c.is_x {
            1
        } else {
            c.amount.max(0) as usize
        };
        match c.source {
            CostSource::Hand => hand_need += amount,
            CostSource::Mill => mill_need += amount,
            CostSource::Graveyard => gy_need += amount,
            CostSource::Sacrifice => {
                for _ in 0..amount {
                    sac_slots.push(c.kind);
                }
            }
            CostSource::Attached => attached_need += amount,
            // P.5: SELF is trivially affordable — you're the resource.
            // No need to count or cap; resolution routes the cast to
            // EXILE instead of its kind's default destination.
            CostSource::SelfExile => {}
        }
    }
    let hand_red = state.cost_reduction(iid, CostSource::Hand).max(0) as usize;
    let mill_red = state.cost_reduction(iid, CostSource::Mill).max(0) as usize;
    let gy_red = state.cost_reduction(iid, CostSource::Graveyard).max(0) as usize;
    let att_red = state.cost_reduction(iid, CostSource::Attached).max(0) as usize;
    hand_need = hand_need.saturating_sub(hand_red);
    mill_need = mill_need.saturating_sub(mill_red);
    gy_need = gy_need.saturating_sub(gy_red);
    attached_need = attached_need.saturating_sub(att_red);
    // RULES P.24a + P.24c: tapping a same-color jewel on BOARD can
    // substitute for exactly one HAND-source component. If one is
    // available, drop the hand_need by 1. Without this, the picker
    // (and human affordability prompt) treats a card as unplayable
    // when the hand is short by exactly one — even if a jewel could
    // cover it.
    if hand_need > 0 && state.find_jewel_tap_candidate(player, iid).is_some() {
        hand_need -= 1;
    }
    let p = state.player(player);
    // Identity-match: only hand cards sharing ≥1 element of the casting
    // card's identity set (colors ∪ symbol) count toward hand_have.
    // Colorless+no-symbol casts are wildcards; colorless+no-symbol
    // discards are NOT.
    let cast_ident = state.card_identity(iid);
    // C.14: transparent cards can't pay for BOARD-placed casts.
    let cast_is_board_placed = matches!(
        inst.card.kind,
        CardType::Creature | CardType::Artifact | CardType::Environment
    );
    let is_transparent = |h: &InstanceId| -> bool {
        state
            .card_pool
            .get(h)
            .map(|i| {
                i.card
                    .colors
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case("transparent"))
            })
            .unwrap_or(false)
    };
    // Single source of truth — the SAME filter set that
    // resolve_hand_payment applies. Picker and resolver can no
    // longer disagree on which hand cards count as payable. This
    // closes the class of bug where the picker over-counts (e.g.,
    // missing a static restriction filter) and offers a cast the
    // resolver then refuses to fund, producing pick/resolve loops.
    //
    // Notes: `iid` (the cast card) is excluded inside the helper;
    // `cast_is_board_placed` / `is_transparent` / `cant_pay` /
    // identity-match are all applied there.
    let _ = (cast_is_board_placed, &is_transparent, &cast_ident);
    let hand_have_identity = state.eligible_hand_payments(player, iid).len();
    // Clear View-style GY-substitutes can fill HAND slots without
    // identity matching. Count eligible cards in GY and add their
    // capacity to the affordability calculation. They cover slots the
    // hand can't satisfy via identity.
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
    let hand_have = hand_have_identity + gy_subs_available;
    // P.12b identity-coverage gate (matches game/play.rs:359-363):
    // when the cast has any identity (colors ∪ symbols), substitutes
    // alone cannot fund payment unless the GY pitch supplies a
    // color-anchor (which suspends P.7a per P.12b). The picker must
    // mirror this: at least ONE actual hand-payment card is required.
    //
    // Without this gate the picker offers casts that fund via
    // substitutes only and play_card then rejects with
    // NoHandPaymentForIdentity → pick/resolve loop (observed with
    // midnight-raven when B's only blue eligible payment was a
    // gy_sub clear-view).
    //
    // Skipped when: cast has empty identity (wildcard), or hand_need
    // is 0 (no hand payment required at all), or a GY anchor will be
    // supplied (gy_need > 0 and at least one color-matching GY card
    // exists — checked just below as the standard P.12a guard).
    if hand_need > 0 && !cast_ident.is_empty() && hand_have_identity == 0 {
        // Would the GY anchor save us via P.12b?
        let cast_colors_lc: std::collections::BTreeSet<String> = inst
            .card
            .colors
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let gy_anchor_possible = gy_need > 0
            && !cast_colors_lc.is_empty()
            && p.graveyard.iter().any(|gid| {
                state
                    .card_pool
                    .get(gid)
                    .map(|i| {
                        i.card
                            .colors
                            .iter()
                            .any(|c| cast_colors_lc.contains(&c.to_ascii_lowercase()))
                    })
                    .unwrap_or(false)
            });
        if !gy_anchor_possible {
            return false;
        }
    }
    // Mirror build_pattern_b_choices's sacrifice filter: a creature with
    // the "can't be sacrificed" keyword (P.31 restriction) is invisible
    // to sacrifice payment. Without this filter the picker over-counts
    // (every board creature looks sacrificable) and build then finds
    // zero eligible — play_card rejects with WrongSacrificeCount and the
    // pick/resolve loop spins (observed on mortal-bee + red-devil).
    let mut available: Vec<InstanceId> = p
        .board
        .iter()
        .filter(|iid| !state.has_keyword(iid, "can't be sacrificed"))
        .cloned()
        .collect();
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
    // Use the shared eligibility helper so the picker can't over-
    // count attached payments the resolver will refuse — closes the
    // C.14 transparent-attached-vs-board-placed-cast disagreement
    // that produced AttachedPaymentInvalid loops on hollow + clear-*.
    let attached_have: usize = state.eligible_attached_payments(player, iid).len();
    let _ = p;
    // P.12a: a cast with non-empty colors and a GRAVEYARD cost component
    // requires at least one color-matching card in GY (the anchor). Without
    // this gate the picker burns rolls on casts play_card refuses with
    // NoGraveyardPaymentForColor → response-window spin.
    if gy_need > 0 {
        let cast_colors: std::collections::BTreeSet<String> = inst
            .card
            .colors
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if !cast_colors.is_empty() {
            let has_anchor = p.graveyard.iter().any(|gid| {
                state
                    .card_pool
                    .get(gid)
                    .map(|i| {
                        i.card
                            .colors
                            .iter()
                            .any(|c| cast_colors.contains(&c.to_ascii_lowercase()))
                    })
                    .unwrap_or(false)
            });
            if !has_anchor {
                return false;
            }
        }
    }
    hand_have >= hand_need
        && p.deck.len() >= mill_need
        && p.graveyard.len() >= gy_need
        && attached_have >= attached_need
        && sac_ok
}

/// Sim heuristic: how valuable to KEEP this attached card vs spend
/// it as P.31 ATTACHED-source payment? Higher = more valuable to keep
/// = sorted later in the pick order. Weights are placeholders pending
/// EA tuning — signals are fixed, magnitudes are guesses.
pub fn attached_keep_value(state: &GameState, attached_iid: &InstanceId) -> i32 {
    let Some(inst) = state.card_pool.get(attached_iid) else {
        return 0;
    };
    let mut score: i32 = 0;
    // (1) Spending a mutation loses its P.28 effect on the host.
    if inst.card.kind == CardType::Mutation || inst.card.static_def.is_some() {
        score += 20;
    }
    // (2) Host crystal tap-substitution (P.24b): an attached card is
    // "load-bearing" for a crystal if removing it would drop the crystal
    // to zero shared-color attached for some color. Approximation: penalize
    // attached cards on crystal hosts where this is the only attached
    // sharing each of its colors.
    if let Some(host) = state.host_of(attached_iid).and_then(|h| state.card_pool.get(&h)) {
        let is_crystal = host
            .card
            .subtypes
            .iter()
            .any(|s| s.eq_ignore_ascii_case("crystal"));
        if is_crystal {
            let my_colors: std::collections::BTreeSet<String> = inst
                .card
                .colors
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .collect();
            for color in &my_colors {
                let sharers = host
                    .attached
                    .iter()
                    .filter(|x| *x != attached_iid)
                    .filter_map(|x| state.card_pool.get(x))
                    .filter(|i| {
                        i.card
                            .colors
                            .iter()
                            .any(|c| c.eq_ignore_ascii_case(color))
                    })
                    .count();
                if sharers == 0 {
                    score += 10;
                }
            }
        }
        // (3) Static-granted activated ability via A.10. Spending the
        // source card strips the granted ability from the host.
        if inst
            .card
            .static_def
            .as_ref()
            .is_some_and(|d| d.granted_activated.is_some())
        {
            score += 15;
        }
        // (4) Shell redundancy: dilute per attached on the host.
        // More crowded hosts mean lower marginal shell value per card.
        let host_attached = host.attached.len().max(1) as i32;
        score += 5 / host_attached;
    }
    score
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
    (x + y).round() as i32 + cost_weight * 2 + attached_count * 2
}

/// Engine-mirror legality check: can `blocker` block `attacker`?
/// Mirrors the keyword half of `combat.rs::declare_blocker` (untapped,
/// not cannot-block, attacker not unblockable, flying needs flying/reach).
/// Subtype overrides (`can_block_subtypes` — cats-block-birds) and
/// subtype prohibitions (`cannot_block_subtypes` — rats-can't-block-cats)
/// are intentionally NOT modelled here. The AI is slightly aggressive
/// against subtype-override blockers (will swing a flyer into a ground
/// cat that engine lets block) and slightly conservative against
/// subtype-prohibited rats (treats them as legal). Both edge cases live
/// in two cards today.
fn can_block_attacker(
    state: &GameState,
    attacker: &InstanceId,
    blocker: &InstanceId,
) -> bool {
    let Some(blk_inst) = state.card_pool.get(blocker) else {
        return false;
    };
    if blk_inst.tapped {
        return false;
    }
    if state.has_keyword(blocker, "cannot-block") {
        return false;
    }
    if state.has_keyword(attacker, "unblockable") {
        return false;
    }
    if state.has_keyword(attacker, "flying")
        && !state.has_keyword(blocker, "flying")
        && !state.has_keyword(blocker, "reach")
    {
        return false;
    }
    true
}

/// Picks the subset of eligible attackers to actually declare this turn.
/// Walks attackers biggest-X first and reserves the defender's clean-kill
/// blockers for top threats — leaving smaller attackers to face thinner
/// boards (or none). Per attacker:
///   - unblockable → swing.
///   - no legal blockers left → swing (mill pressure).
///   - clean-kill block exists (attacker dies, blocker survives) → skip;
///     reserve that blocker so weaker attackers don't see it.
///   - kill-trade option (mutual death) → mirror `pick_blocks` T2 gate
///     for what the defender will actually take, then swing iff WE
///     trade up (defender's blocker is worth ≥5 more than our attacker).
///   - otherwise (bounce / no-block) → swing.
pub fn select_attackers(state: &GameState, player: PlayerId) -> Vec<InstanceId> {
    use std::collections::BTreeSet;

    // O8: emit AttackerSelection event at exit with eligible + chosen.
    let trace_active = crate::trace::is_enabled();
    let t0 = trace_active.then(std::time::Instant::now);

    let attackers = eligible_attackers(state, player);
    if attackers.is_empty() {
        if let Some(t0) = t0 {
            crate::trace::push(crate::trace::TraceEvent::AttackerSelection {
                at_us: crate::trace::now_us(),
                player,
                eligible: Vec::new(),
                chosen: Vec::new(),
                duration_us: t0.elapsed().as_micros() as u64,
            });
        }
        return Vec::new();
    }
    let defender = player.opponent();

    let mut sorted: Vec<(InstanceId, f32, f32, i32)> = attackers
        .iter()
        .map(|a| {
            let (x, y) = state.effective_stats(a);
            let val = sacrifice_keep_value(state, a);
            (a.clone(), x, y, val)
        })
        .collect();
    // f32 has no Ord (NaN); partial_cmp is fine because the engine
    // never produces NaN stats.
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut reserved: BTreeSet<InstanceId> = BTreeSet::new();
    let mut chosen: Vec<InstanceId> = Vec::new();

    for (atk, ax, ay, atk_val) in &sorted {
        if state.has_keyword(atk, "unblockable") {
            chosen.push(atk.clone());
            continue;
        }

        let avail: Vec<(InstanceId, f32, f32, i32)> = state
            .player(defender)
            .board
            .iter()
            .filter(|b| !reserved.contains(*b))
            .filter(|b| can_block_attacker(state, atk, b))
            .map(|b| {
                let (bx, by) = state.effective_stats(b);
                let bval = sacrifice_keep_value(state, b);
                (b.clone(), bx, by, bval)
            })
            .collect();

        if avail.is_empty() {
            chosen.push(atk.clone());
            continue;
        }

        let clean_kill = avail
            .iter()
            .filter(|(_, bx, by, _)| *bx >= *ay && *by > *ax)
            .min_by_key(|(_, _, _, bval)| *bval)
            .cloned();
        if let Some((blk, _, _, _)) = clean_kill {
            reserved.insert(blk);
            continue;
        }

        let kill_trade = avail
            .iter()
            .filter(|(_, bx, _, _)| *bx >= *ay)
            .min_by_key(|(_, _, _, bval)| *bval)
            .cloned();
        if let Some((blk, _, _, bval)) = kill_trade {
            let defender_takes = *ax >= 2.0 || *atk_val > bval + 4;
            if defender_takes {
                if bval > *atk_val + 4 {
                    chosen.push(atk.clone());
                }
                reserved.insert(blk);
                continue;
            }
            chosen.push(atk.clone());
            continue;
        }

        chosen.push(atk.clone());
    }

    if let Some(t0) = t0 {
        crate::trace::push(crate::trace::TraceEvent::AttackerSelection {
            at_us: crate::trace::now_us(),
            player,
            eligible: attackers,
            chosen: chosen.clone(),
            duration_us: t0.elapsed().as_micros() as u64,
        });
    }
    chosen
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
            // RULES B.1: only creatures attack. Artifacts /
            // environments / mutations don't.
            if inst.card.kind != CardType::Creature {
                return false;
            }
            if inst.tapped {
                return false;
            }
            if state.has_keyword(iid, "defender") {
                return false;
            }
            if inst.summoning_sick && !state.has_keyword(iid, "haste") {
                return false;
            }
            if state.has_restriction(iid, crate::card::Restriction::CannotAttack) {
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
    use crate::game::CombatState;

    // O8: emit BlockerSelection event at every exit.
    let trace_active = crate::trace::is_enabled();
    let t0 = trace_active.then(std::time::Instant::now);

    let declared: Vec<InstanceId> = match &state.combat {
        Some(CombatState::AwaitingBlockers { attacks }) => {
            attacks.iter().map(|a| a.attacker.clone()).collect()
        }
        _ => {
            emit_blocker_selection(defender, Vec::new(), Vec::new(), t0);
            return Vec::new();
        }
    };
    if declared.is_empty() {
        emit_blocker_selection(defender, Vec::new(), Vec::new(), t0);
        return Vec::new();
    }

    let blockers = eligible_blockers(state, defender);
    if blockers.is_empty() {
        emit_blocker_selection(defender, declared, Vec::new(), t0);
        return Vec::new();
    }

    // B.2b: defender mills floor(ΣX) per combat, so the dying check
    // applies the floor too.
    let total_incoming_f: f32 = declared
        .iter()
        .map(|a| state.effective_stats(a).0.max(0.0))
        .sum();
    let total_incoming: i32 = total_incoming_f.floor() as i32;
    let deck = state.player(defender).deck.len() as i32;
    let dying = total_incoming >= deck;

    let mut sorted: Vec<(InstanceId, f32, f32)> = declared
        .iter()
        .map(|a| {
            let (x, y) = state.effective_stats(a);
            (a.clone(), x, y)
        })
        .collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut assignments: Vec<(InstanceId, InstanceId)> = Vec::new();
    let mut used: BTreeSet<InstanceId> = BTreeSet::new();
    let mut remaining_incoming = total_incoming;

    for (atk, atk_x, atk_y) in &sorted {
        let avail: Vec<(InstanceId, f32, f32)> = blockers
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
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .cloned();
        if let Some((blk, _, _)) = clean_kill {
            assignments.push((blk.clone(), atk.clone()));
            used.insert(blk);
            remaining_incoming -= atk_x.floor() as i32;
            continue;
        }

        // T2: kill-trade with trade-up.
        let kill_trade = avail
            .iter()
            .filter(|(_, bx, _)| *bx >= *atk_y)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .cloned();
        if let Some((blk, _, _)) = kill_trade {
            let trade_up =
                sacrifice_keep_value(state, atk) > sacrifice_keep_value(state, &blk) + 4;
            if dying || *atk_x >= 2.0 || trade_up {
                assignments.push((blk.clone(), atk.clone()));
                used.insert(blk);
                remaining_incoming -= atk_x.floor() as i32;
                continue;
            }
        }

        // T4: multi-block (dying only).
        if dying {
            let mut by_x = avail.clone();
            by_x.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let mut combined_x = 0.0_f32;
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
                remaining_incoming -= atk_x.floor() as i32;
                continue;
            }
        }

        // T1: chump only if still dying.
        if remaining_incoming >= deck {
            let chump = avail
                .iter()
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .cloned();
            if let Some((blk, _, _)) = chump {
                assignments.push((blk.clone(), atk.clone()));
                used.insert(blk);
                remaining_incoming -= atk_x.floor() as i32;
                continue;
            }
        }
    }

    emit_blocker_selection(defender, declared, assignments.clone(), t0);
    assignments
}

/// O8: shared BlockerSelection emission. No-op when `t0` is None
/// (trace was off at function entry).
fn emit_blocker_selection(
    defender: PlayerId,
    attackers: Vec<InstanceId>,
    assignments: Vec<(InstanceId, InstanceId)>,
    t0: Option<std::time::Instant>,
) {
    let Some(t0) = t0 else { return };
    crate::trace::push(crate::trace::TraceEvent::BlockerSelection {
        at_us: crate::trace::now_us(),
        defender,
        attackers,
        assignments,
        duration_us: t0.elapsed().as_micros() as u64,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use crate::card::{Card, CardType, Stats};

    fn card_creature(id: &str, x: f32, y: f32) -> Card {
        Card {
            id: id.to_string(),
            name: String::new(),
            colors: vec![],
            kind: CardType::Creature,
            timing: None,
            subtypes: vec![],
            cannot_block_subtypes: vec![],
            can_block_subtypes: vec![],
            symbols: Vec::new(),
            frame: None,
            holes: Vec::new(),
            symbol_slots: std::collections::BTreeMap::new(),
            color_slots: std::collections::BTreeMap::new(),
            face: Vec::new(),
            cost: vec![],
            abilities: vec![],
            flavor: String::new(),
            stats: Some(Stats { x, y }),
            static_def: None,
            handlers: BTreeMap::new(),
            activated: vec![],
            gy_hand_substitute: false,
            allow_x_zero: false,
            target: None,
            is_variant: false,
            variant_of: None,
        }
    }

    fn starter_deck(n: usize, prefix: &str) -> Vec<Card> {
        (0..n)
            .map(|i| card_creature(&format!("{prefix}-{i}"), 1.0, 1.0))
            .collect()
    }

    fn fresh() -> GameState {
        GameState::new(starter_deck(60, "a"), starter_deck(60, "b"))
    }

    /// Pull a hand card, overwrite its Card payload with our stats,
    /// move to board, clear summoning sickness.
    fn make_creature(
        state: &mut GameState,
        side: PlayerId,
        id: &str,
        x: f32,
        y: f32,
    ) -> InstanceId {
        let iid = state.player(side).hand[0].clone();
        let inst = state.card_pool.get_mut(&iid).unwrap();
        inst.card = card_creature(id, x, y);
        inst.summoning_sick = false;
        state.player_mut(side).hand.retain(|x| x != &iid);
        state.player_mut(side).board.push(iid.clone());
        iid
    }

    fn add_ability(state: &mut GameState, iid: &InstanceId, ability: &str) {
        state
            .card_pool
            .get_mut(iid)
            .unwrap()
            .card
            .abilities
            .push(ability.to_string());
    }

    #[test]
    fn skips_attacker_facing_clean_kill_blocker() {
        let mut s = fresh();
        make_creature(&mut s, PlayerId::A, "a-1-1", 1.0, 1.0);
        make_creature(&mut s, PlayerId::B, "b-5-5", 5.0, 5.0);
        let chosen = select_attackers(&s, PlayerId::A);
        assert!(chosen.is_empty(), "1/1 should not swing into 5/5");
    }

    #[test]
    fn unblockable_attacker_swings_through_clean_kill() {
        let mut s = fresh();
        let atk = make_creature(&mut s, PlayerId::A, "a-1-1", 1.0, 1.0);
        make_creature(&mut s, PlayerId::B, "b-5-5", 5.0, 5.0);
        add_ability(&mut s, &atk, "unblockable");
        let chosen = select_attackers(&s, PlayerId::A);
        assert_eq!(chosen, vec![atk]);
    }

    #[test]
    fn flyer_swings_past_ground_blocker() {
        let mut s = fresh();
        let atk = make_creature(&mut s, PlayerId::A, "a-flyer", 2.0, 2.0);
        make_creature(&mut s, PlayerId::B, "b-ground", 5.0, 5.0);
        add_ability(&mut s, &atk, "flying");
        let chosen = select_attackers(&s, PlayerId::A);
        assert_eq!(chosen, vec![atk], "flyer should swing past ground 5/5");
    }

    #[test]
    fn reach_blocker_grounds_the_flyer() {
        let mut s = fresh();
        let atk = make_creature(&mut s, PlayerId::A, "a-flyer", 2.0, 2.0);
        let blk = make_creature(&mut s, PlayerId::B, "b-reach", 5.0, 5.0);
        add_ability(&mut s, &atk, "flying");
        add_ability(&mut s, &blk, "reach");
        let chosen = select_attackers(&s, PlayerId::A);
        assert!(chosen.is_empty(), "reach 5/5 should clean-kill 2/2 flyer");
    }

    #[test]
    fn weaker_attacker_swings_when_big_threat_reserves_blocker() {
        // A's 5/5 faces B's 6/6 clean-kill → 5/5 reserves blocker, 1/1
        // sees empty board and swings for the mill.
        let mut s = fresh();
        let _big = make_creature(&mut s, PlayerId::A, "a-5-5", 5.0, 5.0);
        let small = make_creature(&mut s, PlayerId::A, "a-1-1", 1.0, 1.0);
        make_creature(&mut s, PlayerId::B, "b-6-6", 6.0, 6.0);
        let chosen = select_attackers(&s, PlayerId::A);
        assert_eq!(chosen, vec![small], "small should slip past reserved blocker");
    }

    #[test]
    fn tapped_blocker_is_ignored() {
        let mut s = fresh();
        let atk = make_creature(&mut s, PlayerId::A, "a-1-1", 1.0, 1.0);
        let blk = make_creature(&mut s, PlayerId::B, "b-5-5", 5.0, 5.0);
        s.card_pool.get_mut(&blk).unwrap().tapped = true;
        let chosen = select_attackers(&s, PlayerId::A);
        assert_eq!(chosen, vec![atk], "tapped blocker should not deter attack");
    }

    #[test]
    fn no_swing_when_no_attackers() {
        let s = fresh();
        let chosen = select_attackers(&s, PlayerId::A);
        assert!(chosen.is_empty());
    }
}
