//! Sim AI heuristics. Pure-state functions plus the picker used by the
//! `run_game` loop. No mutation of GameStats — all writes happen in
//! [`super::run`].

use rand::seq::SliceRandom;
use rand::Rng;
use crate::card::{CardType, CostSource};
use crate::cast_routing::CastRouting;
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
            let is_creature = inst.card().kind == CardType::Creature;
            match kind_filter {
                PickKindFilter::Any => {}
                PickKindFilter::CreatureOnly if !is_creature => return false,
                PickKindFilter::NonCreatureOnly if is_creature => return false,
                _ => {}
            }
            match inst.card().kind {
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
                    // CannotBeAttachedTo). C.14's frame gate is lifted.
                    !state.eligible_mutation_targets(iid).is_empty()
                }
                // Typeless casts (P.1 default to GRAVEYARD; SelfExile
                // shortcut to EXILE). Affordability-gated like spells —
                // SELF is trivially payable per `can_pay_instant_cost`,
                // so the Clear cycle gets picked here.
                CardType::Unspecified => can_pay_instant_cost(state, player, iid),
                // C.17 / P.37: Symbol cards are board-placed permanents
                // — affordability-gated like a creature/artifact, plus
                // the P.35 one-per-turn cap + P.36 uniqueness checks
                // already gate them inside play_card. Without this arm
                // they fell through to `_ => false` and the AI never
                // offered Symbols, leaving every drawn Symbol dead in
                // hand.
                CardType::Symbol => can_pay_instant_cost(state, player, iid),
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
            .map(|i| i.card().id.clone())
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
    if let Some(def) = &inst.card().static_def {
        let mut has_cost = false;
        let mut has_stat_or_keyword = false;
        let mut has_restrict = false;
        for eff in &def.effects {
            match eff {
                crate::card::StaticEffect::CostModify { .. } => has_cost = true,
                crate::card::StaticEffect::StatBoost { x, y } => {
                    let nonzero = !matches!(x, crate::ModifierValue::Fixed(n) if *n == 0.0)
                        || !matches!(y, crate::ModifierValue::Fixed(n) if *n == 0.0);
                    if nonzero {
                        has_stat_or_keyword = true;
                    }
                }
                crate::card::StaticEffect::KeywordGrant(_) => has_stat_or_keyword = true,
                crate::card::StaticEffect::Restrict(_) => has_restrict = true,
                _ => {}
            }
        }
        if has_cost {
            s += 50;
        }
        if has_stat_or_keyword {
            s += 20;
        }
        if has_restrict {
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
    if let Some(target) = inst.card().target {
        if !state.is_target_legal(target) {
            return false;
        }
    }
    // RULES P.36: Symbol uniqueness in play. If a Symbol with the
    // same card.id is on either player's BOARD, the engine refuses
    // the cast — picker must mirror or it offers iids play_card
    // immediately rejects with SymbolUniquenessViolated. Without
    // this, UCT rollouts spin on duplicate-Symbol picks and the
    // failure sink fills with per-rollout backtrace dumps.
    if matches!(inst.card().kind, CardType::Symbol) {
        let cast_id = inst.card().id.clone();
        let already_in_play = state
            .a
            .board
            .iter()
            .chain(state.b.board.iter())
            .any(|bid| {
                state
                    .card_pool
                    .get(bid)
                    .map(|i| i.card().id == cast_id)
                    .unwrap_or(false)
            });
        if already_in_play {
            return false;
        }
    }
    // RULES P.35: only one Symbol cast per player per turn. Picker
    // mirrors the play_card gate (game/play.rs sets
    // SymbolCastCapReached) so duplicate-cast attempts in the same
    // turn don't churn the rollout.
    if matches!(inst.card().kind, CardType::Symbol) {
        let idx = match player {
            PlayerId::A => 0,
            PlayerId::B => 1,
        };
        if state.symbol_cast_this_turn[idx] {
            return false;
        }
    }
    let mut hand_need = 0usize;
    let mut mill_need = 0usize;
    let mut gy_need = 0usize;
    let mut attached_need = 0usize;
    let mut tap_need = 0usize;
    let mut sac_slots: Vec<Option<CardType>> = Vec::new();
    // Variable-X handling: an is_x component contributes X * (component
    // amount, typically 1) to its source's need. The AI doesn't pick X
    // here — that happens in the play loop via oracle.choose_int. For
    // affordability, treat is_x as needing 1 of the resource minimum:
    // the cast is "useful" iff at least X=1 is payable. X=0 makes the
    // cast a no-op, so we don't bother accepting cards we'd cast for X=0.
    for c in &inst.card().cost {
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
            // P.42: count tap needs; affordability (enough untapped
            // permanents + a P.42a color anchor) is checked after the
            // reductions below, against the same shared eligibility set the
            // builder and resolver use.
            CostSource::Tap => tap_need += amount,
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
    let tap_red = state.cost_reduction(iid, CostSource::Tap).max(0) as usize;
    tap_need = tap_need.saturating_sub(tap_red);
    // P.42 tap affordability. `tap` isn't substitutable (P.24 covers only
    // HAND/GRAVEYARD), so gate it before the jewel logic: need enough
    // untapped permanents to tap, plus a satisfiable P.42a color anchor.
    // Matches what `resolve_tap_payment` builds and `play_card` validates,
    // so the picker can't offer a tap cast the resolver refuses.
    if tap_need > 0 {
        let tappable = state.eligible_tap_payments(player);
        if tappable.len() < tap_need {
            return false;
        }
        let cast_colors: std::collections::BTreeSet<String> = inst
            .card()
            .colors
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        // A colorless cast can never anchor (P.42a) → unpayable with tap.
        if cast_colors.is_empty() {
            return false;
        }
        let oncolor = |cand: &InstanceId| -> bool {
            state
                .card_pool
                .get(cand)
                .map(|i| {
                    i.card()
                        .colors
                        .iter()
                        .any(|c| cast_colors.contains(&c.to_ascii_lowercase()))
                })
                .unwrap_or(false)
        };
        // Anchor may come from any payment source (P.42a cross-source): a
        // GRAVEYARD component auto-anchors via P.12a; a HAND component can
        // anchor with an on-color card; otherwise an on-color tap is
        // required (the tap-only case).
        let anchor_ok = gy_need > 0
            || tappable.iter().any(|t| oncolor(t))
            || (hand_need > 0
                && state
                    .player(player)
                    .hand
                    .iter()
                    .any(|h| h != iid && oncolor(h)));
        if !anchor_ok {
            return false;
        }
    }
    // RULES P.24a (rewritten) + P.24c: an untapped same-color jewel
    // on BOARD substitutes for UP TO TWO cost components from HAND
    // and/or GRAVEYARD in any combination. Mirror the engine's
    // greedy split at game/play.rs's P.24a apply site — drain HAND
    // first then GRAVEYARD until the 2-component budget is spent —
    // so the picker and build never disagree on coverage. Without
    // this, the picker still applied the OLD "jewel = 1 hand only"
    // shape and refused casts the resolver would accept (2-hand
    // creatures + 1 jewel, 1-hand + 1-gy + 1 jewel, etc.), surfaced
    // as picker/build asymmetry inside UCT rollouts.
    if let Some(sub_iid) = state.find_jewel_tap_candidate(player, iid) {
        // `find_jewel_tap_candidate` is overloaded: it returns either a
        // JEWEL (P.24a — covers up to 2 HAND-and/or-GRAVEYARD components)
        // OR a CRYSTAL (P.24b — covers exactly 1 HAND component, no
        // graveyard). Differentiating by subtype matches the engine's
        // apply site at game/play.rs:285. Without this, witch-bat
        // (1-hand + 1-gy purple creature) cast against a same-color
        // crystal on board had the picker crediting 2-mixed coverage
        // (gy_need → 0, P.12a anchor check skipped) while the engine
        // processed crystal as 1-hand-only (gy_need stayed at 1,
        // anchor check fired, NoGraveyardPaymentForColor 20K times
        // in a 200-game curve-sample run).
        let is_crystal = state
            .card_pool
            .get(&sub_iid)
            .map(|i| i.card().subtypes.iter().any(|s| s.eq_ignore_ascii_case("crystal")))
            .unwrap_or(false);
        if hand_need > 0 || gy_need > 0 {
            if is_crystal {
                // P.24b: crystal covers exactly 1 HAND component.
                if hand_need > 0 {
                    hand_need = hand_need.saturating_sub(1);
                }
            } else {
                // P.24a: jewel covers up to 2 mixed HAND/GRAVEYARD.
                let mut budget: usize = 2;
                let take_h = hand_need.min(budget);
                hand_need -= take_h;
                budget -= take_h;
                let take_g = gy_need.min(budget);
                gy_need -= take_g;
            }
        }
    } else if (hand_need > 0 || gy_need > 0)
        && state.find_symbol_tap_candidate(player).is_some()
    {
        // P.24e: an untapped Symbol on the controller's BOARD
        // substitutes for exactly ONE component (HAND or GRAVEYARD),
        // no color requirement. P.24c caps a cast at one substitution
        // mechanism — credited only when no jewel/crystal took the slot.
        if hand_need > 0 {
            hand_need = hand_need.saturating_sub(1);
        } else {
            gy_need = gy_need.saturating_sub(1);
        }
    }
    let p = state.player(player);
    // Identity-match: only hand cards sharing ≥1 element of the casting
    // card's identity set (colors ∪ symbol) count toward hand_have.
    // Colorless+no-symbol casts are wildcards; colorless+no-symbol
    // discards are NOT.
    let cast_ident = state.card_identity(iid);
    // C.14: transparent cards can't pay for BOARD-placed casts.
    let cast_is_board_placed = inst.card().kind.is_board_placed();
    let is_transparent = |h: &InstanceId| -> bool {
        state
            .card_pool
            .get(h)
            .map(|i| {
                i.card()
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
                .map(|i| i.card().gy_hand_substitute)
                .unwrap_or(false)
        })
        .count();
    // Z.8c: cardless sleeves in hand are non-anchor bodies — like Clear
    // View GY-substitutes, they fill HAND slots the identity cards can't.
    // Only counted for an identity cast: for a wildcard cast they already
    // count inside hand_have_identity (empty identity = every payment
    // matches), so adding them again would double-count. The anchor gate
    // below still requires hand_have_identity >= 1 for an identity cast,
    // so cardless only ADDS capacity beyond a real anchor — never funds a
    // cast alone (which play_card would reject with NoHandPaymentForIdentity).
    let cardless_bodies = if cast_ident.is_empty() {
        0
    } else {
        p.hand
            .iter()
            .filter(|h| *h != iid && state.is_cardless(h))
            .count()
    };
    let hand_have = hand_have_identity + gy_subs_available + cardless_bodies;
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
            .card()
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
                        i.card()
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
                    .map(|i| i.card().kind == *k)
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
    // Use the shared eligibility helper so the picker and resolver
    // always agree on which attached iids may pay this cast. (C.14's
    // frame gate is lifted; the helper now returns all controlled
    // attached cards.)
    let attached_have: usize = state.eligible_attached_payments(player, iid).len();
    let _ = p;
    // P.12a: a cast with non-empty colors and a GRAVEYARD cost component
    // requires at least one color-matching card in GY (the anchor). Without
    // this gate the picker burns rolls on casts play_card refuses with
    // NoGraveyardPaymentForColor → response-window spin.
    if gy_need > 0 {
        let cast_colors: std::collections::BTreeSet<String> = inst
            .card()
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
                        i.card()
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
    // Z.8c: cardless sleeves are never milled, so mill affordability counts
    // card-bearing sleeves only — mirrors the resolver's real-card count in
    // game/play.rs, or the picker offers a mill cast the resolver then
    // rejects with InsufficientDeckForMill.
    let millable_deck = p.deck.iter().filter(|iid| !state.is_cardless(iid)).count();
    hand_have >= hand_need
        && millable_deck >= mill_need
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
    if inst.card().kind == CardType::Mutation || inst.card().static_def.is_some() {
        score += 20;
    }
    // (2) Host crystal tap-substitution (P.24b): an attached card is
    // "load-bearing" for a crystal if removing it would drop the crystal
    // to zero shared-color attached for some color. Approximation: penalize
    // attached cards on crystal hosts where this is the only attached
    // sharing each of its colors.
    if let Some(host) = state.host_of(attached_iid).and_then(|h| state.card_pool.get(&h)) {
        let is_crystal = host
            .card()
            .subtypes
            .iter()
            .any(|s| s.eq_ignore_ascii_case("crystal"));
        if is_crystal {
            let my_colors: std::collections::BTreeSet<String> = inst
                .card()
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
                        i.card()
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
            .card()
            .static_def
            .as_ref()
            .is_some_and(|d| {
                d.effects
                    .iter()
                    .any(|e| matches!(e, crate::card::StaticEffect::GrantActivated(_)))
            })
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
    let cost_weight: i32 = inst.card().cost.iter().map(|c| c.amount.max(0)).sum();
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
            if inst.card().kind != CardType::Creature {
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
            same_sleeve: false,
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
        inst.content = Some(card_creature(id, x, y));
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
            .card_mut()
            .abilities
            .push(ability.to_string());
    }

    // P.24e: an untapped Symbol on the controller's BOARD substitutes
    // for exactly ONE cost component (HAND or GRAVEYARD), no color
    // requirement. P.24c limits a cast to one substitution mechanism
    // total — the picker credits the jewel first (it covers 2 vs 1),
    // and only credits a Symbol if no jewel is taking the slot.
    //
    // Scenario: 1-hand red creature, no in-hand red payment, no
    // jewel, but a green Symbol untapped on A's board. Old picker
    // ignored the Symbol entirely and refused the cast; the engine
    // would have accepted `choices.jewel_tap = Some(symbol_iid)`.
    #[test]
    fn can_pay_instant_cost_symbol_covers_one_hand_component() {
        use crate::card::{CostComponent, CostSource};
        let mut s = fresh();
        let cast = s.player(PlayerId::A).hand[0].clone();
        let symbol = s.player(PlayerId::A).hand[1].clone();
        {
            let c = s.card_pool.get_mut(&cast).unwrap();
            c.card_mut().colors = vec!["red".to_string()];
            c.card_mut().cost = vec![CostComponent {
                amount: 1,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            }];
        }
        {
            let sym = s.card_pool.get_mut(&symbol).unwrap();
            sym.card_mut().kind = CardType::Symbol;
            // Deliberately a different color from the cast — P.24e has
            // no color requirement.
            sym.card_mut().colors = vec!["green".to_string()];
        }
        s.player_mut(PlayerId::A).hand.retain(|x| x != &symbol);
        s.player_mut(PlayerId::A).board.push(symbol.clone());
        s.card_pool.get_mut(&symbol).unwrap().tapped = false;
        assert!(
            can_pay_instant_cost(&s, PlayerId::A, &cast),
            "Symbol-tap must cover one HAND component per P.24e",
        );
    }

    // P.24e: a Symbol can pay for a GRAVEYARD component, including
    // cases where no color-matching GY anchor exists (the engine
    // skips the anchor check entirely when no GY pitch happens).
    // Setup with a blue GY seed (cast is red) so old picker would
    // refuse via NoGraveyardPaymentForColor; new picker covers the
    // GY slot with the Symbol-tap and accepts.
    #[test]
    fn can_pay_instant_cost_symbol_covers_one_graveyard_component() {
        use crate::card::{CostComponent, CostSource};
        let mut s = fresh();
        let cast = s.player(PlayerId::A).hand[0].clone();
        let symbol = s.player(PlayerId::A).hand[1].clone();
        let gy_seed = s.player(PlayerId::A).hand[2].clone();
        {
            let c = s.card_pool.get_mut(&cast).unwrap();
            c.card_mut().colors = vec!["red".to_string()];
            c.card_mut().cost = vec![CostComponent {
                amount: 1,
                source: CostSource::Graveyard,
                is_x: false,
                kind: None,
            }];
        }
        {
            let sym = s.card_pool.get_mut(&symbol).unwrap();
            sym.card_mut().kind = CardType::Symbol;
            sym.card_mut().colors = vec!["red".to_string()];
        }
        {
            // Blue GY seed — does NOT anchor a red cast. Without
            // Symbol-tap coverage the gy_anchor gate refuses.
            s.card_pool.get_mut(&gy_seed).unwrap().card_mut().colors = vec!["blue".to_string()];
        }
        s.player_mut(PlayerId::A).hand.retain(|x| x != &symbol);
        s.player_mut(PlayerId::A).board.push(symbol.clone());
        s.card_pool.get_mut(&symbol).unwrap().tapped = false;
        s.player_mut(PlayerId::A).hand.retain(|x| x != &gy_seed);
        s.player_mut(PlayerId::A).graveyard.push(gy_seed);
        assert!(
            can_pay_instant_cost(&s, PlayerId::A, &cast),
            "Symbol-tap must cover the GRAVEYARD slot so the anchor check is skipped",
        );
    }

    // make pool surfaced [play_card-ERR] cast=surge err=WrongHandPaymentCount
    // { expected: 1, got: 0 } with B's hand=[chaos-dragon, surge,
    // clear-green]. surge is a 2-hand blue instant; B's hand has no
    // blue card and B's gy has no clear-* substitutes — so eligible
    // hand payments are empty. The P.7a identity gate at ai.rs:319
    // SHOULD refuse: hand_need > 0, cast_ident = {blue} non-empty,
    // hand_have_identity = 0, no gy_anchor possible (surge has no
    // GRAVEYARD-source component). Picker offering anyway → build
    // can't fill the slot → engine errors. This test pins the
    // minimal repro at the picker.
    #[test]
    fn can_pay_instant_cost_refuses_2hand_spell_with_no_identity_match_in_hand() {
        use crate::card::{CostComponent, CostSource};
        let mut s = fresh();
        let cast = s.player(PlayerId::A).hand[0].clone();
        let off_color_a = s.player(PlayerId::A).hand[1].clone();
        let off_color_b = s.player(PlayerId::A).hand[2].clone();
        {
            // 2-hand blue spell, mirroring surge.
            let c = s.card_pool.get_mut(&cast).unwrap();
            c.card_mut().colors = vec!["blue".to_string()];
            c.card_mut().kind = CardType::Spell;
            c.card_mut().cost = vec![CostComponent {
                amount: 2,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            }];
        }
        // Other hand cards: red + green — neither shares blue's
        // identity, so eligible_hand_payments returns empty.
        s.card_pool.get_mut(&off_color_a).unwrap().card_mut().colors = vec!["red".to_string()];
        s.card_pool.get_mut(&off_color_b).unwrap().card_mut().colors = vec!["green".to_string()];
        // No jewel / symbol on board, no clear-* substitutes in gy.
        assert!(
            !can_pay_instant_cost(&s, PlayerId::A, &cast),
            "2-hand spell with no identity-matching hand card and no GY substitutes / jewel / symbol must be refused by the picker",
        );
    }

    // Same setup as above + a Spell-affecting hand-cost-reduction
    // static on the controller's BOARD. The engine sees raw 2 - red 1 = 1
    // hand_needed; build can't fill the slot (no eligible hand pay,
    // no GY sub); engine errors `WrongHandPaymentCount { expected: 1,
    // got: 0 }`. The picker MUST still refuse this state: after
    // hand_red reduces hand_need to 1, hand_have_identity is 0 and no
    // gy_anchor possible — gate at ai.rs:319 fires. If the gate is
    // skipping for some reason (this is the surge repro from
    // make pool), the picker accepts and the test fails.
    #[test]
    fn can_pay_instant_cost_refuses_2hand_spell_with_static_hand_reduction_and_no_identity_match() {
        use crate::card::{
            CardType, CostComponent, CostSource, StaticAffects,
            StaticDef,
        };
        let mut s = fresh();
        let cast = s.player(PlayerId::A).hand[0].clone();
        let off_color_a = s.player(PlayerId::A).hand[1].clone();
        let off_color_b = s.player(PlayerId::A).hand[2].clone();
        let reducer = s.player(PlayerId::A).hand[3].clone();
        {
            // 2-hand blue spell.
            let c = s.card_pool.get_mut(&cast).unwrap();
            c.card_mut().colors = vec!["blue".to_string()];
            c.card_mut().kind = CardType::Spell;
            c.card_mut().cost = vec![CostComponent {
                amount: 2,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            }];
        }
        s.card_pool.get_mut(&off_color_a).unwrap().card_mut().colors = vec!["red".to_string()];
        s.card_pool.get_mut(&off_color_b).unwrap().card_mut().colors = vec!["green".to_string()];
        // Static that reduces Spell hand cost by 1. Mirrors modern-
        // lcd-clock's shape but targets Spell kind instead of Creature.
        {
            let r = s.card_pool.get_mut(&reducer).unwrap();
            r.card_mut().kind = CardType::Artifact;
            r.card_mut().static_def = Some(StaticDef {
                affects: StaticAffects {
                    kind: Some(CardType::Spell),
                    ..Default::default()
                },
                condition: None,
                effects: vec![crate::card::StaticEffect::CostModify {
                    source: CostSource::Hand,
                    amount: 1,
                }],
            });
        }
        // Reducer goes to BOARD so its static fires.
        s.player_mut(PlayerId::A).hand.retain(|x| x != &reducer);
        s.player_mut(PlayerId::A).board.push(reducer.clone());
        assert!(
            !can_pay_instant_cost(&s, PlayerId::A, &cast),
            "even with a -1 hand static reducing cost to 1, picker must refuse a blue spell when no identity-matching pay exists and no GY anchor possible",
        );
    }

    // P.24a (rewritten): the engine's jewel substitution now covers
    // UP TO TWO cost components from HAND and/or GRAVEYARD. The
    // picker must mirror that or it will refuse casts the resolver
    // would happily fund — surfaced as picker/build asymmetry in
    // the EA's UCT rollouts.
    //
    // Scenario: 2-hand red creature in hand, A's hand has no other
    // red card (identity_count = 0), one red jewel untapped on
    // board. Old picker reduced hand_need by 1 → still 1 → identity
    // gate refused. New picker should cover both with the jewel and
    // accept.
    #[test]
    fn can_pay_instant_cost_jewel_covers_two_hand_components() {
        use crate::card::{CostComponent, CostSource};
        let mut s = fresh();
        let cast = s.player(PlayerId::A).hand[0].clone();
        let jewel = s.player(PlayerId::A).hand[1].clone();
        {
            let c = s.card_pool.get_mut(&cast).unwrap();
            c.card_mut().colors = vec!["red".to_string()];
            c.card_mut().cost = vec![CostComponent {
                amount: 2,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            }];
        }
        {
            let j = s.card_pool.get_mut(&jewel).unwrap();
            j.card_mut().kind = CardType::Artifact;
            j.card_mut().colors = vec!["red".to_string()];
            j.card_mut().subtypes = vec!["jewel".to_string()];
        }
        // Move the jewel to A's board, untapped.
        s.player_mut(PlayerId::A).hand.retain(|x| x != &jewel);
        s.player_mut(PlayerId::A).board.push(jewel.clone());
        s.card_pool.get_mut(&jewel).unwrap().tapped = false;
        assert!(
            can_pay_instant_cost(&s, PlayerId::A, &cast),
            "jewel must cover both HAND components per rewritten P.24a",
        );
    }

    // P.24a (rewritten) — mixed HAND + GRAVEYARD coverage. The jewel
    // pays one HAND and one GRAVEYARD component; without this the
    // picker would refuse a cast the engine resolves cleanly.
    #[test]
    fn can_pay_instant_cost_jewel_covers_one_hand_one_graveyard_mixed() {
        use crate::card::{CostComponent, CostSource};
        let mut s = fresh();
        let cast = s.player(PlayerId::A).hand[0].clone();
        let jewel = s.player(PlayerId::A).hand[1].clone();
        // Seed a red graveyard card so the gy_need path has material;
        // and so the gy-color-anchor check won't NoGraveyardPaymentForColor.
        let gy_seed = s.player(PlayerId::A).hand[2].clone();
        {
            let c = s.card_pool.get_mut(&cast).unwrap();
            c.card_mut().colors = vec!["red".to_string()];
            c.card_mut().cost = vec![
                CostComponent {
                    amount: 1,
                    source: CostSource::Hand,
                    is_x: false,
                    kind: None,
                },
                CostComponent {
                    amount: 1,
                    source: CostSource::Graveyard,
                    is_x: false,
                    kind: None,
                },
            ];
        }
        {
            let j = s.card_pool.get_mut(&jewel).unwrap();
            j.card_mut().kind = CardType::Artifact;
            j.card_mut().colors = vec!["red".to_string()];
            j.card_mut().subtypes = vec!["jewel".to_string()];
        }
        {
            let g = s.card_pool.get_mut(&gy_seed).unwrap();
            g.card_mut().colors = vec!["red".to_string()];
        }
        s.player_mut(PlayerId::A).hand.retain(|x| x != &jewel);
        s.player_mut(PlayerId::A).board.push(jewel.clone());
        s.card_pool.get_mut(&jewel).unwrap().tapped = false;
        s.player_mut(PlayerId::A).hand.retain(|x| x != &gy_seed);
        s.player_mut(PlayerId::A).graveyard.push(gy_seed);
        assert!(
            can_pay_instant_cost(&s, PlayerId::A, &cast),
            "jewel must cover one HAND + one GRAVEYARD slot per rewritten P.24a",
        );
    }

    // C.17 / P.37: a Symbol card in hand with no cost (the canonical
    // shape from the 50-card grid) must be offered as a playable pick
    // by the heuristic enumerator. Prior to this fix, Symbol fell
    // through the picker's `_ => false` arm and every AI treated
    // Symbols as dead draws.
    #[test]
    fn enumerate_playable_in_hand_offers_symbol_cards() {
        let mut s = fresh();
        let iid = s.player(PlayerId::A).hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&iid).unwrap();
            inst.card_mut().kind = CardType::Symbol;
            inst.card_mut().cost = vec![];
        }
        let offered = enumerate_playable_in_hand(&s, PlayerId::A, PickKindFilter::Any);
        assert!(
            offered.iter().any(|i| i == &iid),
            "Symbol card in hand must appear in playable enumeration; got {offered:?}",
        );
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
