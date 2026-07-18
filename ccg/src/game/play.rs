//! Playing a card from hand: cost payment, destination, attachment.
//!
//! Mirrors RULES.md P.1, P.2, P.6, P.7, P.11, P.17.

mod activate;
mod errors;
mod payments;

use super::context::EventContext;
use super::lua_api;
use super::state::{DeathReplacement, GameState, InstanceId, PlayerId, StackItem, Zone};
use crate::card::{CardType, CostSource, EventName};
use crate::cast_routing::CastRouting;
use crate::choice::ResponseAction;
use std::collections::BTreeSet;

// Re-exports: PlayChoices/PlayError are referenced in this file;
// ActivateError isn't (it moved to play/activate.rs) but play_tests.rs
// imports it from this module path.
#[allow(unused_imports)]
pub use errors::ActivateError;
pub use errors::{ActivateChoices, PlayChoices, PlayError};

impl GameState {
    /// Play `instance` from `player`'s HAND, paying its cost via `choices`.
    ///
    /// Atomic: returns Err and leaves state unchanged if any validation fails.
    /// Mutations only occur after all checks pass.
    ///
    /// Restrictions in this slice:
    ///   - Only CREATURE cards.
    ///   - Only `HAND` and `MILL` cost sources.
    ///   - No variable X (`is_x: true` returns Err).
    ///   - No timing checks (caller is responsible for C.6 / C.10 / U.7 / U.8).
    ///
    /// On success:
    ///   - MILL cost paid (top N of player's DECK → GRAVEYARD, per P.11).
    ///   - HAND payments removed from hand.
    ///   - Played card moved from HAND to BOARD (per P.2).
    ///   - HAND payments attached to the played card (per P.6), face-down (per P.17).
    pub fn play_card(
        &mut self,
        player: PlayerId,
        instance: &InstanceId,
        choices: PlayChoices,
        ctx: Option<&mut EventContext>,
    ) -> Result<(), PlayError> {
        // O4: bracket the entire body with `Instant::now()` + emit
        // a single Play event on exit, regardless of early returns.
        let trace_active = crate::trace::is_enabled();
        let t0 = trace_active.then(std::time::Instant::now);
        let iid_for_trace = trace_active.then(|| instance.clone());

        // Permanent diagnostic — capture the cast snapshot BEFORE
        // play_card_inner runs so it survives the call even when
        // play_card_inner Errs after consuming `choices`.
        let cast_snapshot = self.card_pool.get(instance).map(|i| i.card().id.clone());
        let hand_snapshot: Vec<String> = self
            .player(player)
            .hand
            .iter()
            .filter_map(|h| self.card_pool.get(h).map(|i| i.card().id.clone()))
            .collect();
        let gy_snapshot: Vec<String> = self
            .player(player)
            .graveyard
            .iter()
            .filter_map(|h| self.card_pool.get(h).map(|i| i.card().id.clone()))
            .collect();
        let x_snapshot = choices.x_value;
        let hand_pay_snapshot: Vec<String> = choices
            .hand_payment_ids
            .iter()
            .filter_map(|h| self.card_pool.get(h).map(|i| i.card().id.clone()))
            .collect();
        let gy_pay_snapshot: Vec<String> = choices
            .gy_hand_payment_ids
            .iter()
            .filter_map(|h| self.card_pool.get(h).map(|i| i.card().id.clone()))
            .collect();
        let result = self.play_card_inner(player, instance, choices, ctx);

        // Errors are sacred. Every play_card failure captures its
        // full triangulation — cast, player, choices, hand, GY, AND
        // the caller's backtrace so the call origin of a residual
        // picker/resolver disagreement is recoverable without
        // re-instrumenting. Goes through the per-thread failure
        // sink (Vec push, no stderr lock), drained at game end.
        if let Err(err) = &result {
            let cast = cast_snapshot.unwrap_or_default();
            let bt = std::backtrace::Backtrace::force_capture();
            crate::sim::instrument::push_failure(format!(
                "[play_card-ERR] cast={cast} player={player:?} \
                 err={err:?} x_value={x_snapshot:?} \
                 hand_payment_ids=[{}] gy_hand_payment_ids=[{}] \
                 hand=[{}] graveyard=[{}]\nbacktrace:\n{bt}",
                hand_pay_snapshot.join(", "),
                gy_pay_snapshot.join(", "),
                hand_snapshot.join(", "),
                gy_snapshot.join(", "),
            ));
        }

        if let (Some(t0), Some(iid)) = (t0, iid_for_trace) {
            // PlayError::ChoicePending is a SUSPEND (the engine
            // catches it and yields a HumanPrompt) — not a failure.
            // Tagging it as Err here was the LOG misclassification
            // that made Fireball look like a crash in trace v1.
            let outcome = match &result {
                Ok(()) => crate::trace::OutcomeRepr::Ok,
                Err(crate::game::PlayError::ChoicePending(p)) => {
                    crate::trace::OutcomeRepr::Suspend(format!("{p:?}"))
                }
                Err(e) => crate::trace::OutcomeRepr::Err(format!("{e:?}")),
            };
            crate::trace::push(crate::trace::TraceEvent::Play {
                at_us: crate::trace::now_us(),
                iid,
                outcome,
                duration_us: t0.elapsed().as_micros() as u64,
            });
        }
        result
    }

    fn play_card_inner(
        &mut self,
        player: PlayerId,
        instance: &InstanceId,
        choices: PlayChoices,
        ctx: Option<&mut EventContext>,
    ) -> Result<(), PlayError> {
        let mut ctx = ctx;
        if self.winner.is_some() {
            return Err(PlayError::GameOver);
        }

        // P.38: a Symbol card on top of its controller's DECK is
        // castable from there as if it were in HAND. Every cast-time
        // check (P.32 / P.35 / P.36 / timing / cost) still runs; only
        // the source-zone selection differs. The cast iid leaves DECK
        // (not HAND) at announcement and resolves to BOARD per P.37.
        let inst_ref = self.card_pool.get(instance).ok_or(PlayError::NotInHand)?;
        let card_kind = inst_ref.card().kind;
        let card_cost = inst_ref.card().cost.clone();
        let from_hand = self.player(player).hand.contains(instance);
        let from_top_of_deck = matches!(card_kind, CardType::Symbol)
            && self.player(player).deck.first() == Some(instance);
        if !from_hand && !from_top_of_deck {
            return Err(PlayError::NotInHand);
        }
        let cast_source_zone = if from_hand {
            Zone::Hand
        } else {
            Zone::Deck
        };

        if !card_kind.is_castable() {
            // TODO(types): Environment (→ BOARD per P.21 + P.22 slot management).
            return Err(PlayError::UnsupportedType(card_kind));
        }
        // P.35: per-turn Symbol cap. Checked here so the cast is
        // refused before any cost is paid and the card stays in HAND
        // (mirroring CastValidateFailed semantics). The cap is per-
        // player: the flag is set in resolve_played_card_inner when
        // the cast actually completes, so a self-exile Symbol or any
        // future early-exit doesn't burn the player's one slot.
        let symbol_idx = match player {
            super::state::PlayerId::A => 0,
            super::state::PlayerId::B => 1,
        };
        if matches!(card_kind, CardType::Symbol)
            && self.symbol_cast_this_turn[symbol_idx]
        {
            return Err(PlayError::SymbolCastCapReached);
        }
        // P.36: Symbol uniqueness in play. A second cast with the same
        // card-`id` while a Symbol carrying that id is on either player's
        // BOARD is refused before any cost is paid. Scope is BOARD only
        // (not GRAVEYARD / EXILE / HAND): once the first leaves the
        // BOARD the id is castable again.
        if matches!(card_kind, CardType::Symbol) {
            let cast_card_id = inst_ref.card().id.clone();
            let any_on_board = self
                .a
                .board
                .iter()
                .chain(self.b.board.iter())
                .any(|iid| {
                    self.card_pool
                        .get(iid)
                        .map(|i| i.card().id == cast_card_id)
                        .unwrap_or(false)
                });
            if any_on_board {
                return Err(PlayError::SymbolUniquenessViolated);
            }
        }
        // Sorcery timing: a Spell with Timing::Sorcery cannot be cast while
        // a response window is open (main-phase only).
        let card_timing = inst_ref.card().timing;
        if card_timing == Some(crate::card::Timing::Sorcery) && self.priority.is_some() {
            return Err(PlayError::SorceryAtInstantSpeed);
        }

        // RULES P.32: declarative target category. If the card declares
        // a target category and no legal target exists, refuse the cast
        // before any state mutation. Counterspell uses `target = "chain"`
        // to refuse when the stack is empty.
        if let Some(target) = inst_ref.card().target {
            if !self.is_target_legal(target) {
                return Err(PlayError::CastValidateFailed);
            }
        }

        // Aggregate cost requirements per source.
        let mut hand_needed: usize = 0;
        let mut mill_needed: usize = 0;
        let mut graveyard_needed: usize = 0;
        let mut sacrifice_needed: usize = 0;
        let mut attached_needed: usize = 0;
        // Variable-X: if any cost component has is_x, the player must have
        // pre-chosen X (via oracle.choose_int) and supplied it in choices.
        // The same X applies to every variable component.
        let has_variable_x = card_cost.iter().any(|c| c.is_x);
        let allow_x_zero = self
            .card_pool
            .get(instance)
            .map(|i| i.card().allow_x_zero)
            .unwrap_or(false);
        let x_value = if has_variable_x {
            match choices.x_value {
                Some(v) => {
                    if v < 1 && !allow_x_zero {
                        return Err(PlayError::XBelowMinimum);
                    }
                    v.max(0) as usize
                }
                None => return Err(PlayError::VariableXValueMissing),
            }
        } else {
            0
        };

        for c in &card_cost {
            let amount = if c.is_x {
                x_value
            } else {
                c.amount.max(0) as usize
            };
            match c.source {
                CostSource::Hand => hand_needed += amount,
                CostSource::Mill => mill_needed += amount,
                CostSource::Graveyard => graveyard_needed += amount,
                CostSource::Sacrifice => sacrifice_needed += amount,
                CostSource::Attached => attached_needed += amount,
                CostSource::SelfExile => {
                    // P.5: routing handled in resolve_played_card_inner;
                    // here the component is trivially satisfied (the
                    // cast card itself is the resource).
                    let _ = amount;
                }
            }
        }

        // Phase 3.5 cost-modification pre-pass: each on-board static whose
        // `affects` matches the cast card can reduce per-source costs (per
        // CostModifier entries). P.20 clamps each component to 0 minimum.
        let hand_red = self.cost_reduction(instance, CostSource::Hand).max(0) as usize;
        let mill_red = self.cost_reduction(instance, CostSource::Mill).max(0) as usize;
        let gy_red = self.cost_reduction(instance, CostSource::Graveyard).max(0) as usize;
        let sac_red = self
            .cost_reduction(instance, CostSource::Sacrifice)
            .max(0) as usize;
        hand_needed = hand_needed.saturating_sub(hand_red);
        mill_needed = mill_needed.saturating_sub(mill_red);
        graveyard_needed = graveyard_needed.saturating_sub(gy_red);
        sacrifice_needed = sacrifice_needed.saturating_sub(sac_red);
        let att_red = self
            .cost_reduction(instance, CostSource::Attached)
            .max(0) as usize;
        attached_needed = attached_needed.saturating_sub(att_red);

        // P.24: validate optional jewel-tap. Pull card colors once for both
        // the jewel-color check (here) and any future uses. After validation,
        // reduce `hand_needed` by 1 — the jewel substitutes for one HAND slot.
        let cast_card_colors: Vec<String> = self
            .card_pool
            .get(instance)
            .map(|i| {
                i.card()
                    .colors
                    .iter()
                    .map(|c| c.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        // P.24a / P.24b / P.24e share the `choices.jewel_tap` slot,
        // implementing the P.24c "at most one substitution per cast"
        // rule structurally (one field — pick at most one mechanism).
        // The three differ in coverage and side-effects:
        //   - jewel (P.24a):  tap + sacrifice; up to 2 components,
        //                     HAND and/or GRAVEYARD; color overlap
        //                     with cast required.
        //   - crystal (P.24b): tap; exactly 1 HAND component; one of
        //                     the crystal's attached cards must share
        //                     a color with the cast.
        //   - symbol (P.24e):  tap; exactly 1 HAND or GRAVEYARD
        //                     component; no color requirement, just
        //                     untapped + controller's BOARD.
        if let Some(sub_iid) = &choices.jewel_tap {
            let (sub_is_jewel, sub_is_crystal, sub_is_symbol) = self
                .card_pool
                .get(sub_iid)
                .map(|i| {
                    let subs = &i.card().subtypes;
                    (
                        subs.iter().any(|s| s.eq_ignore_ascii_case("jewel")),
                        subs.iter().any(|s| s.eq_ignore_ascii_case("crystal")),
                        matches!(i.card().kind, CardType::Symbol),
                    )
                })
                .unwrap_or((false, false, false));
            if sub_is_jewel || sub_is_crystal {
                let valid = self.is_valid_jewel_tap(player, sub_iid, &cast_card_colors);
                if !valid {
                    return Err(PlayError::InvalidJewelTap(sub_iid.clone()));
                }
            } else if sub_is_symbol {
                // No color overlap requirement. Just: on this
                // player's BOARD, controlled by them, untapped.
                let valid = self
                    .card_pool
                    .get(sub_iid)
                    .map(|i| {
                        !i.tapped
                            && i.controller == player
                            && self.player(player).board.contains(sub_iid)
                    })
                    .unwrap_or(false);
                if !valid {
                    return Err(PlayError::InvalidJewelTap(sub_iid.clone()));
                }
            } else {
                return Err(PlayError::InvalidJewelTap(sub_iid.clone()));
            }
            // Coverage budget.
            if sub_is_jewel {
                if hand_needed == 0 && graveyard_needed == 0 {
                    return Err(PlayError::JewelTapWithoutHandCost);
                }
                let mut budget: usize = 2;
                let hand_take = hand_needed.min(budget);
                hand_needed -= hand_take;
                budget -= hand_take;
                let gy_take = graveyard_needed.min(budget);
                graveyard_needed -= gy_take;
            } else if sub_is_crystal {
                // P.24b: crystal substitutes for exactly one HAND
                // component, no GRAVEYARD coverage.
                if hand_needed == 0 {
                    return Err(PlayError::JewelTapWithoutHandCost);
                }
                hand_needed -= 1;
            } else {
                // P.24e (Symbol): one component, HAND preferred,
                // GRAVEYARD if no HAND need.
                if hand_needed == 0 && graveyard_needed == 0 {
                    return Err(PlayError::JewelTapWithoutHandCost);
                }
                if hand_needed > 0 {
                    hand_needed -= 1;
                } else {
                    graveyard_needed -= 1;
                }
            }
        }

        // P.12a + P.12b color-anchor on GRAVEYARD-source payments.
        // When the cast has a GRAVEYARD cost component and non-empty
        // colors, at least one card being exiled to pay it must share
        // a printed color with the cast (lenient — one anchor for the
        // whole bundle suffices). When the anchor is supplied, P.12b
        // suspends P.7a's identity check on HAND payments for this cast.
        //
        // Smart auto-pitch: when the caller leaves
        // `choices.graveyard_payment_ids` empty, the engine builds the
        // pitch list itself. The legacy strategy of "take the last N
        // cards from GY" caused picker/play_card disagreement — the
        // picker (sim/ai.rs:340) checks whether *any* card in GY can
        // anchor, but the engine then pitched the back-of-GY cards
        // which often didn't anchor, returning NoGraveyardPaymentForColor.
        // That gap caused rollout hangs (e.g., glass-damselfly
        // re-picked forever). The smart strategy: take one anchor
        // (the most-recent color-matching card) plus the rest from
        // back-of-GY, matching the picker's optimism.
        //
        // The computed `auto_gy_pitch` is also used at exile-time
        // below so the check and the actual move stay coherent.
        let cast_colors_set: BTreeSet<String> = cast_card_colors.iter().cloned().collect();
        let auto_gy_pitch: Vec<InstanceId> = if graveyard_needed > 0
            && choices.graveyard_payment_ids.is_empty()
        {
            let gy = &self.player(player).graveyard;
            if cast_colors_set.is_empty() {
                // Empty-color cast: anchor moot, back-of-GY is fine.
                let start = gy.len().saturating_sub(graveyard_needed);
                gy[start..].to_vec()
            } else {
                // Find the most-recent card whose printed colors share
                // anything with the cast. Lowercase-fold both sides.
                let anchor_idx: Option<usize> = (0..gy.len()).rev().find(|&i| {
                    self.card_pool
                        .get(&gy[i])
                        .map(|inst| {
                            inst.card().colors.iter().any(|c| {
                                cast_colors_set.contains(&c.to_ascii_lowercase())
                            })
                        })
                        .unwrap_or(false)
                });
                let mut chosen: Vec<usize> = Vec::with_capacity(graveyard_needed);
                if let Some(a) = anchor_idx {
                    chosen.push(a);
                }
                // Fill remaining slots from the back of GY, skipping
                // the already-picked anchor.
                for i in (0..gy.len()).rev() {
                    if chosen.len() >= graveyard_needed {
                        break;
                    }
                    if !chosen.contains(&i) {
                        chosen.push(i);
                    }
                }
                // Exile order: most-recent first (matches the legacy
                // back-of-GY loop's order for the non-anchor entries
                // and keeps journal locality predictable).
                chosen.sort_by(|a, b| b.cmp(a));
                chosen.iter().map(|&i| gy[i].clone()).collect()
            }
        } else {
            Vec::new()
        };
        let gy_supplies_color_anchor = if graveyard_needed > 0 {
            if cast_colors_set.is_empty() {
                // Empty-color cast is a wildcard already; anchor moot.
                true
            } else {
                let pitch_ids: Vec<InstanceId> = if !choices.graveyard_payment_ids.is_empty() {
                    choices.graveyard_payment_ids.clone()
                } else {
                    auto_gy_pitch.clone()
                };
                let mut found = false;
                for gid in &pitch_ids {
                    let pay_colors: BTreeSet<String> = self
                        .card_pool
                        .get(gid)
                        .map(|i| {
                            i.card()
                                .colors
                                .iter()
                                .map(|c| c.to_ascii_lowercase())
                                .collect()
                        })
                        .unwrap_or_default();
                    if cast_colors_set.iter().any(|c| pay_colors.contains(c)) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(PlayError::NoGraveyardPaymentForColor);
                }
                true
            }
        } else {
            false
        };

        // Clear View-style GY → EXILE substitutes. Each one fills one
        // HAND slot without going through the P.7a identity check.
        // Validate each: must be in controller's GY, must have the
        // substitute flag, must be unique within the list. Then
        // subtract them from hand_needed before the hand_payment count
        // check.
        if !choices.gy_hand_payment_ids.is_empty() {
            let total_hand_pre_subst = hand_needed + choices.gy_hand_payment_ids.len();
            if total_hand_pre_subst == choices.gy_hand_payment_ids.len() && hand_needed == 0 {
                return Err(PlayError::GyHandSubstituteWithoutHandCost);
            }
            let mut gy_seen: BTreeSet<&InstanceId> = BTreeSet::new();
            for gid in &choices.gy_hand_payment_ids {
                if !gy_seen.insert(gid) {
                    return Err(PlayError::DuplicateGyHandSubstitute(gid.clone()));
                }
                if !self.player(player).graveyard.contains(gid) {
                    return Err(PlayError::GyHandSubstituteNotInGraveyard(gid.clone()));
                }
                let eligible = self
                    .card_pool
                    .get(gid)
                    .map(|i| i.card().gy_hand_substitute)
                    .unwrap_or(false);
                if !eligible {
                    return Err(PlayError::GyHandSubstituteNotEligible(gid.clone()));
                }
            }
            if choices.gy_hand_payment_ids.len() > hand_needed {
                return Err(PlayError::WrongHandPaymentCount {
                    expected: hand_needed,
                    got: choices.gy_hand_payment_ids.len(),
                });
            }
            hand_needed -= choices.gy_hand_payment_ids.len();
        }

        if choices.hand_payment_ids.len() != hand_needed {
            return Err(PlayError::WrongHandPaymentCount {
                expected: hand_needed,
                got: choices.hand_payment_ids.len(),
            });
        }

        // Identity-coverage gate (per RULES P.7a + Clear View design):
        // if the cast has any identity (colors or symbols) and any of
        // the HAND slots were filled by GY substitutes, at least one
        // HAND-payment must come from hand. Substitutes don't carry
        // identity, so an all-substitute payment leaves identity
        // uncovered.
        if !choices.gy_hand_payment_ids.is_empty()
            && choices.hand_payment_ids.is_empty()
            && !gy_supplies_color_anchor
        {
            let cast_ident = self.card_identity(instance);
            if !cast_ident.is_empty() {
                // Errors are sacred — the engine knows exactly which
                // state produced this rejection and the operator
                // must see it. Captured into the per-thread failure
                // sink (drained at game end) so the per-error cost
                // is a string format + Vec push, not a stderr lock.
                let card_id = self
                    .card_pool
                    .get(instance)
                    .map(|i| i.card().id.clone())
                    .unwrap_or_default();
                let ids = |zone: &[InstanceId]| -> Vec<String> {
                    zone.iter()
                        .map(|h| {
                            self.card_pool
                                .get(h)
                                .map(|i| i.card().id.clone())
                                .unwrap_or_else(|| h.clone())
                        })
                        .collect()
                };
                let eligible_now = self.eligible_hand_payments(player, instance);
                let cast_ident_vec: Vec<String> = cast_ident.iter().cloned().collect();
                crate::sim::instrument::push_failure(format!(
                    "[NoHandPaymentForIdentity] cast={card_id} player={player:?} \
                     x_value={:?} cast_ident={cast_ident_vec:?} \
                     eligible_hand_payments=[{}]  gy_hand_substitutes=[{}]  \
                     hand_payment_ids=[{}]  hand=[{}]  graveyard=[{}]",
                    choices.x_value,
                    ids(&eligible_now).join(", "),
                    ids(&choices.gy_hand_payment_ids).join(", "),
                    ids(&choices.hand_payment_ids).join(", "),
                    ids(&self.player(player).hand).join(", "),
                    ids(&self.player(player).graveyard).join(", "),
                ));
                return Err(PlayError::NoHandPaymentForIdentity);
            }
        }

        // Z.8c: cardless sleeves are HAND-cost bodies with no identity. If
        // the cast has identity, no GY color anchor was supplied, and EVERY
        // HAND payment is a cardless body, none anchors identity — reject
        // (parallel to the substitute gate above; a real matching card must
        // anchor, cardless bodies fill the rest).
        if !gy_supplies_color_anchor
            && !choices.hand_payment_ids.is_empty()
            && choices.hand_payment_ids.iter().all(|h| self.is_cardless(h))
            && !self.card_identity(instance).is_empty()
        {
            return Err(PlayError::NoHandPaymentForIdentity);
        }

        let mut seen: BTreeSet<&InstanceId> = BTreeSet::new();
        for hid in &choices.hand_payment_ids {
            // Duplicate detection is a set-level rule and stays here; the
            // per-item legality (not-self, in-hand, P.24, P.7a-with-P.12b-
            // and-Z.8c-exemptions) is the single shared predicate the
            // eligibility helpers filter on, so the picker can never offer
            // a payment this loop refuses.
            if !seen.insert(hid) {
                return Err(PlayError::DuplicateHandPayment(hid.clone()));
            }
            if let Some(e) = self.hand_payment_item_error(
                instance,
                hid,
                player,
                gy_supplies_color_anchor,
                true,
            ) {
                return Err(e);
            }
        }

        // Z.8c: only card-bearing sleeves can be milled, so affordability
        // counts real cards, not cardless sleeves that would be skimmed.
        let deck_have = self
            .player(player)
            .deck
            .iter()
            .filter(|iid| !self.is_cardless(iid))
            .count();
        if deck_have < mill_needed {
            return Err(PlayError::InsufficientDeckForMill {
                needed: mill_needed,
                have: deck_have,
            });
        }

        let gy_have = self.player(player).graveyard.len();
        if gy_have < graveyard_needed {
            return Err(PlayError::InsufficientGraveyardForCost {
                needed: graveyard_needed,
                have: gy_have,
            });
        }

        // P.12 explicit-id path: when the caller supplies
        // `graveyard_payment_ids`, validate count/membership/uniqueness.
        // Empty is the legacy fallback (back-of-GY) and is not an error.
        if !choices.graveyard_payment_ids.is_empty() {
            if choices.graveyard_payment_ids.len() != graveyard_needed {
                return Err(PlayError::WrongGraveyardPaymentCount {
                    expected: graveyard_needed,
                    got: choices.graveyard_payment_ids.len(),
                });
            }
            let mut gy_seen: BTreeSet<&InstanceId> = BTreeSet::new();
            for gid in &choices.graveyard_payment_ids {
                if !gy_seen.insert(gid) {
                    return Err(PlayError::DuplicateGraveyardPayment(gid.clone()));
                }
                if !self.player(player).graveyard.contains(gid) {
                    return Err(PlayError::GraveyardPaymentInvalid(gid.clone()));
                }
            }
        }

        // Mutation target validation: a Mutation cast must name a creature
        // on either BOARD to attach to. Any creature qualifies — except
        // ones with a `CannotBeAttachedTo` restriction (glass-insect cycle).
        if matches!(card_kind, CardType::Mutation) {
            // Target presence is set-level (the choice must exist); the
            // per-item legality (on-board creature, not CannotBeAttachedTo,
            // sleeve not full) is the shared predicate the picker's
            // eligible_mutation_targets filters on.
            let Some(target) = &choices.mutation_target else {
                return Err(PlayError::MutationTargetMissing);
            };
            if let Some(e) = self.mutation_target_item_error(target) {
                return Err(e);
            }
        }

        // P.16: SACRIFICE cost validation. Each chosen sacrifice ID must be
        // on the player's BOARD, controlled by them, and (if the cost
        // component specifies a kind) match that kind. Caller chooses
        // which board cards to sacrifice (sim AI prefers low-value targets).
        //
        // Multiple SACRIFICE cost components on one card are matched by
        // order — the i-th sacrifice_id pairs with the i-th SACRIFICE
        // component for kind-filter checking. Today no card has more than
        // one SACRIFICE component, so this is forward-looking.
        if choices.sacrifice_ids.len() != sacrifice_needed {
            return Err(PlayError::WrongSacrificeCount {
                expected: sacrifice_needed,
                got: choices.sacrifice_ids.len(),
            });
        }
        let sac_kinds: Vec<Option<CardType>> = card_cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Sacrifice))
            .flat_map(|c| {
                let n = if c.is_x { x_value } else { c.amount.max(0) as usize };
                std::iter::repeat_n(c.kind, n)
            })
            .collect();
        let mut sac_seen: BTreeSet<&InstanceId> = BTreeSet::new();
        for (i, sid) in choices.sacrifice_ids.iter().enumerate() {
            if !sac_seen.insert(sid) {
                return Err(PlayError::DuplicateSacrifice(sid.clone()));
            }
            if !self.player(player).board.contains(sid) {
                return Err(PlayError::SacrificePaymentInvalid(sid.clone()));
            }
            let Some(inst) = self.card_pool.get(sid) else {
                return Err(PlayError::SacrificePaymentInvalid(sid.clone()));
            };
            if inst.controller != player {
                return Err(PlayError::SacrificePaymentInvalid(sid.clone()));
            }
            if let Some(required_kind) = sac_kinds.get(i).copied().flatten() {
                if inst.card().kind != required_kind {
                    return Err(PlayError::SacrificePaymentInvalid(sid.clone()));
                }
            }
        }

        // P.31: ATTACHED-source payment validation. Each id must currently
        // be attached to a card the player controls on the BOARD; no dups;
        // count must match `attached_needed`.
        if choices.attached_payment_ids.len() != attached_needed {
            return Err(PlayError::WrongAttachedPaymentCount {
                expected: attached_needed,
                got: choices.attached_payment_ids.len(),
            });
        }
        let mut att_seen: BTreeSet<&InstanceId> = BTreeSet::new();
        for aid in &choices.attached_payment_ids {
            // Duplicate detection stays here (set-level); host-on-your-
            // BOARD legality is the shared predicate eligible_attached_
            // payments filters on. (C.14 lifted: frame no longer gates it.)
            if !att_seen.insert(aid) {
                return Err(PlayError::DuplicateAttachedPayment(aid.clone()));
            }
            if let Some(e) = self.attached_payment_item_error(aid, player) {
                return Err(e);
            }
        }

        // All checks pass — apply mutations through journaled helpers.

        // Capture payment-id snapshot for `game.payment_ids()`. Hand and
        // attached are caller-supplied via `choices`; gy and mill we
        // collect during this resolution. Cleared after OnPlay fires.
        let mut payments_snapshot = super::state::CastPayments {
            hand: choices.hand_payment_ids.clone(),
            attached: choices.attached_payment_ids.clone(),
            graveyard: Vec::new(),
            mill: Vec::new(),
            sacrifice: choices.sacrifice_ids.clone(),
        };

        // MILL cost: top N CARD-bearing sleeves → GRAVEYARD (P.11). Z.8c: a
        // cardless sleeve never counts for mill — it is skimmed to GY along
        // the way without consuming a mill slot (parallel to the Z.8b draw
        // skip).
        let mut milled = 0;
        while milled < mill_needed {
            let Some(top) = self.player(player).deck.first().cloned() else {
                break;
            };
            let is_cardless = self.is_cardless(&top);
            // Sacred-error sweep: `top` came from `deck.first()` so
            // NotInZone shouldn't be possible — but if it ever happens,
            // it's state corruption that previously hid silently.
            let _ = self.move_card_or_emit(
                &top,
                player,
                Zone::Deck,
                Zone::Graveyard,
                "play-mill-cost",
            );
            if !is_cardless {
                payments_snapshot.mill.push(top.clone());
                milled += 1;
            }
        }

        // GRAVEYARD cost (P.12): if the caller supplied explicit
        // `graveyard_payment_ids`, exile those in order. Otherwise use
        // the smart auto-pitch computed up-front (anchor-first when
        // the cast has colors, back-of-GY otherwise). Using the same
        // list as the P.12a check keeps the two coherent — no more
        // picker/play_card disagreement on which cards anchor.
        if choices.graveyard_payment_ids.is_empty() {
            for gid in &auto_gy_pitch {
                payments_snapshot.graveyard.push(gid.clone());
                let _ = self.move_card_or_emit(
                    gid,
                    player,
                    Zone::Graveyard,
                    Zone::Exile,
                    "play-gy-cost-auto",
                );
            }
        } else {
            payments_snapshot.graveyard = choices.graveyard_payment_ids.clone();
            for gid in choices.graveyard_payment_ids.clone() {
                let _ = self.move_card_or_emit(
                    &gid,
                    player,
                    Zone::Graveyard,
                    Zone::Exile,
                    "play-gy-cost-explicit",
                );
            }
        }

        // Stash the payment snapshot on state so OnPlay handlers can read
        // it via `game.payment_ids()`. resolve_played_card_inner clears
        // it after OnPlay fires. Survives the stack-resolve hop between
        // play_card (here) and resolve_played_card_inner.
        self.current_cast_payments = Some(payments_snapshot);

        // P.24: cost-substitution apply. All three mechanisms tap the
        // source. Only the jewel (P.24a) is additionally sacrificed.
        // Crystal (P.24b) and Symbol (P.24e) stay on the BOARD,
        // tapped, until normal untap (U.2). P.8's attached-cascade
        // still applies to the jewel sacrifice.
        if let Some(sub_iid) = &choices.jewel_tap {
            let sub_iid = sub_iid.clone();
            let is_jewel = self
                .card_pool
                .get(&sub_iid)
                .map(|i| i.card().subtypes.iter().any(|s| s.eq_ignore_ascii_case("jewel")))
                .unwrap_or(false);
            self.set_tapped(&sub_iid, true);
            if is_jewel {
                let _ = self.move_card_or_emit(
                    &sub_iid,
                    player,
                    Zone::Board,
                    Zone::Graveyard,
                    "play-jewel-sacrifice",
                );
            }
            self.bump_action("jewel_tap_substitution", player);
        }

        // Clear View-style HAND-substitute payments: each chosen card
        // in GY moves GY → EXILE. Validation above confirmed eligibility.
        for gid in choices.gy_hand_payment_ids.clone() {
            let _ = self.move_card_or_emit(
                &gid,
                player,
                Zone::Graveyard,
                Zone::Exile,
                "play-gy-hand-substitute",
            );
            self.bump_action("gy_hand_substitution", player);
        }

        // P.16: SACRIFICE — move chosen BOARD cards to GRAVEYARD and fire
        // on_die per card (matches combat's death-detection sequence).
        let sac_ids: Vec<InstanceId> = choices.sacrifice_ids.clone();
        for sid in &sac_ids {
            let _ = self.move_card_or_emit(
                sid,
                player,
                Zone::Board,
                Zone::Graveyard,
                "play-sacrifice-cost",
            );
            self.bump_action("sacrificed_as_cost", player);
        }
        if let Some(c) = ctx.as_mut() {
            for sid in &sac_ids {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnDie, sid)
                    .map_err(PlayError::ChoicePending)?;
                // OnCreatureDies broadcast to BOARD watchers (excludes
                // the dying card — already moved to GRAVEYARD above).
                let watchers: Vec<InstanceId> = self
                    .a
                    .board
                    .iter()
                    .chain(self.b.board.iter())
                    .cloned()
                    .collect();
                for watcher in &watchers {
                    lua_api::fire_with_partner(
                        c.lua,
                        self,
                        c.oracle(),
                        EventName::OnCreatureDies,
                        watcher,
                        sid,
                    )
                    .map_err(PlayError::ChoicePending)?;
                }
            }
        }
        // P.8: cascade attached → EXILE for each sacrificed card, after
        // on_die handlers had their chance to read self.attached.
        for sid in &sac_ids {
            self.exile_remaining_attached(sid);
        }

        // RULES P.33: the cast card itself leaves HAND at cast time. It
        // joins the response chain (transient — no Z-zone backing). On
        // resolution it moves to its destination zone; if countered it
        // moves to GRAVEYARD. Without this, the same card sits in hand
        // through the whole response window and the AI can cast it
        // again on its own turn-of-priority — which lets two players
        // alternate identical responses indefinitely (see Bug 3).
        // HAND payments still stay in hand until resolution (their
        // refund-on-counter semantic is unchanged).
        let _ = self.remove_from_zone_or_emit(
            instance,
            player,
            cast_source_zone,
            "play-cast-source-remove",
        );

        // Announce the cast. Non-hand cost (mill, graveyard) is already
        // paid; HAND payments stay in hand until resolution (mirrors MTG:
        // if the cast gets countered, HAND payments don't leave hand, but
        // mill/graveyard payments don't refund).
        let cast = StackItem::PlayedCard {
            card: instance.clone(),
            controller: player,
            choices,
        };

        // If a response window is already open, this is a cast-in-response:
        // push onto the chain and return. The outer driver (the play_card
        // call that opened the window) will pop and resolve it. If no
        // window is open, this is a normal cast: open one and drive it.
        if self.priority.is_some() {
            self.respond_with(cast)
                .expect("priority.is_some() checked above");
            return Ok(());
        }

        self.open_response_window(cast)
            .expect("priority.is_none() checked above");
        self.drive_window_to_close(ctx)
    }

    /// Drive the currently-open response window to close. At each priority
    /// handoff, asks the oracle (via `ctx`) "respond or pass?". A `Respond`
    /// re-enters `play_card` which routes to `respond_with` (priority is
    /// open). A `Pass` calls `pass_priority`; if that pops an item, the
    /// item is resolved before the loop continues. The loop exits when the
    /// window closes (chain empty + both pass).
    ///
    /// TODO(stack-phase-2-driver): Option B — the right long-term shape is
    /// for `play_card` to just announce, and this loop to live in the
    /// outer caller (sim or UI). That removes the re-entrant `play_card`
    /// call and matches how human-driven play actually works. Option A
    /// (here) is Phase 1 expedience.
    pub fn drive_window_to_close(
        &mut self,
        ctx: Option<&mut EventContext>,
    ) -> Result<(), PlayError> {
        let mut ctx = ctx;
        // Spin-detection: track consecutive Responds that fail
        // play_card. A bug elsewhere (e.g., a respond_or_pass policy
        // that picks an illegal-target card the engine refuses) would
        // otherwise loop here forever — the priority window doesn't
        // advance on failed casts, the oracle re-picks the same card.
        // Cap is generous (50): legitimate response chains never
        // approach it. On trip: dump diagnostics, force-close the
        // window, bump a stat-counter so EA reports surface the event.
        let mut consecutive_failed_responds: u32 = 0;
        let mut last_failed_card: Option<InstanceId> = None;
        let mut last_failed_err: Option<PlayError> = None;
        while self.priority.is_some() && self.winner.is_none() {
            // Chain-overflow tripwire: dump the chain contents at depth
            // 40 so we can see what's piling up (e.g., alternating
            // counterspells, repeated same card) before bounding via a
            // permanent cap. Halts via the shared timeout counter.
            let chain_depth = self
                .priority
                .as_ref()
                .map(|p| p.chain.len())
                .unwrap_or(0);
            if chain_depth >= 40 {
                let chain_dump: Vec<String> = self
                    .priority
                    .as_ref()
                    .map(|p| {
                        p.chain
                            .iter()
                            .enumerate()
                            .map(|(i, StackItem::PlayedCard { card, controller, .. })| {
                                let card_id = self
                                    .card_pool
                                    .get(card)
                                    .map(|inst| inst.card().id.clone())
                                    .unwrap_or_else(|| format!("?{card}"));
                                format!("[{i}] {:?}={}", controller, card_id)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Sacred-error: an invariant violation (chain stack
                // grew unboundedly) needs to land in the dev tool, not
                // only in the native CLI's stderr. Surfaces as
                // surface="engine", region="response-stack-overflow"
                // so the LOG / inline overlay shows the depth + dump.
                crate::error::emit_region(
                    crate::error::Severity::Error,
                    "engine",
                    "response-stack-overflow",
                    format!(
                        "response stack overflowed at depth {chain_depth} (turn={}, active={:?})",
                        self.turn, self.active_player
                    ),
                    format!("chain contents:\n  {}", chain_dump.join("\n  ")),
                );
                eprintln!(
                    "[CHAIN OVERFLOW] turn={} active={:?} chain_len={} contents:\n  {}",
                    self.turn,
                    self.active_player,
                    chain_depth,
                    chain_dump.join("\n  "),
                );
                self.bump_action("chain_overflow", self.active_player);
                let _ = super::bump_timeout_and_maybe_halt(
                    "drive_window_to_close (chain overflow)",
                );
                self.priority = None;
                break;
            }
            if consecutive_failed_responds > 50 {
                let chain_ids: Vec<String> = self
                    .priority
                    .as_ref()
                    .map(|p| {
                        p.chain
                            .iter()
                            .map(|StackItem::PlayedCard { card, .. }| {
                                self.card_pool
                                    .get(card)
                                    .map(|i| i.card().id.clone())
                                    .unwrap_or_else(|| format!("?{card}"))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let failed_card_id = last_failed_card
                    .as_ref()
                    .and_then(|iid| self.card_pool.get(iid).map(|i| i.card().id.clone()))
                    .unwrap_or_else(|| "(unknown)".to_string());
                // Sacred-error: the same "response window can't make
                // progress" infinite-loop guard that surfaces
                // CHAIN OVERFLOW also produces this spin variant.
                // Land it in the dev tool too so the loop's victim
                // (the card whose response keeps failing) is visible.
                crate::error::emit_region(
                    crate::error::Severity::Error,
                    "engine",
                    "response-spin",
                    format!(
                        "response window spun {consecutive_failed_responds}× on the same card (turn={}, active={:?}, last_failed_card={failed_card_id})",
                        self.turn, self.active_player
                    ),
                    format!("last_err={last_failed_err:?} chain={chain_ids:?}"),
                );
                eprintln!(
                    "[RESPONSE SPIN] turn={} active={:?} consecutive_failed_responds={} \
                     last_failed_card={} err={:?} chain={:?}",
                    self.turn,
                    self.active_player,
                    consecutive_failed_responds,
                    failed_card_id,
                    last_failed_err,
                    chain_ids,
                );
                self.bump_action("response_spin_aborted", self.active_player);
                let _ = super::bump_timeout_and_maybe_halt(
                    "drive_window_to_close (response spin)",
                );
                // Force-close the window. The pending chain is dropped —
                // the game continues but this priority sequence is lost.
                self.priority = None;
                break;
            }
            let next = self.priority.as_ref().expect("checked is_some").next_to_act;
            let action = match ctx.as_mut() {
                Some(c) => c.oracle().respond_or_pass(self, next),
                None => ResponseAction::Pass,
            };
            match action {
                ResponseAction::Respond { card, choices } => {
                    self.bump_action("instant_response_played", next);
                    let result = self.play_card(next, &card, choices, ctx.as_deref_mut());
                    if let Err(e) = &result {
                        consecutive_failed_responds += 1;
                        last_failed_card = Some(card.clone());
                        last_failed_err = Some(e.clone());
                    } else {
                        consecutive_failed_responds = 0;
                        last_failed_card = None;
                        last_failed_err = None;
                    }
                }
                ResponseAction::Pass => {
                    // Forward progress (priority advancing) resets the spin counter.
                    consecutive_failed_responds = 0;
                    last_failed_card = None;
                    last_failed_err = None;
                    match self.pass_priority() {
                    Ok(Some(item)) => match item {
                        StackItem::PlayedCard {
                            card,
                            controller,
                            choices,
                        } => {
                            self.resolve_played_card(
                                &card,
                                controller,
                                choices,
                                ctx.as_deref_mut(),
                            )?;
                        }
                    },
                    Ok(None) => continue,
                    Err(_) => return Ok(()),
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolution of a popped `StackItem::PlayedCard`: HAND payments leave
    /// hand, the played card moves to its destination zone, and post-play
    /// triggers fire inline. Mirrors RULES P.1 (instants → GRAVEYARD) and
    /// the creature ETB sequence.
    fn resolve_played_card(
        &mut self,
        instance: &InstanceId,
        player: PlayerId,
        choices: PlayChoices,
        ctx: Option<&mut EventContext>,
    ) -> Result<(), PlayError> {
        let card_kind = self
            .card_pool
            .get(instance)
            .map(|i| i.card().kind)
            .unwrap_or(CardType::Unspecified);
        // Expose the cast-time X value to `OnPlay` handlers via
        // `game.x_value()`. Mirrors the activation path. Saved and
        // restored around the entire resolution match so handlers
        // (and nested ETB / on_attached_as_cost fires) see it
        // consistently. None outside an X-cost cast.
        let prior_x = self.current_activation_x;
        self.current_activation_x = choices.x_value;
        let mut ctx = ctx;
        let result =
            self.resolve_played_card_inner(instance, player, choices, ctx.as_deref_mut(), card_kind);
        self.current_activation_x = prior_x;
        // C.15: after all cast-time handlers have run, scan for any
        // creature whose effective Y has dropped to ≤ 0 (via detach,
        // stat-modifier, attached-source payment, etc.) and move it to
        // GRAVEYARD. Runs even on Err — partial state mutations are
        // ostensibly rolled back via journal, but defense in depth.
        self.cleanup_zero_y_deaths(ctx);
        result
    }

    /// Inner body of `resolve_played_card` split out only so the X-value
    /// guard above can wrap a single expression. The original logic is
    /// unchanged.
    fn resolve_played_card_inner(
        &mut self,
        instance: &InstanceId,
        player: PlayerId,
        choices: PlayChoices,
        ctx: Option<&mut EventContext>,
        card_kind: CardType,
    ) -> Result<(), PlayError> {
        let mut ctx = ctx;
        // P.5: if any cost component is SelfExile (amount > 0), the cast
        // card routes to EXILE on resolution regardless of declared kind.
        // HAND payments fall back to the spell-payment convention
        // (GRAVEYARD) since there's no host on BOARD to attach them to.
        // ATTACHED payments follow the non-BOARD branch (EXILE per P.31).
        // on_play still fires; on_enter_board / OnAttachedAsCost do not.
        let self_exiles = self
            .card_pool
            .get(instance)
            .map(|i| {
                i.card().cost.iter().any(|c| {
                    matches!(c.source, CostSource::SelfExile) && c.amount.max(0) > 0
                })
            })
            .unwrap_or(false);
        if self_exiles {
            for hid in choices.hand_payment_ids.clone() {
                let _ = self.move_card_or_emit(
                    &hid,
                    player,
                    Zone::Hand,
                    Zone::Graveyard,
                    "play-self-exile-hand-pay",
                );
            }
            for aid in choices.attached_payment_ids.clone() {
                if let Some(host) = self.host_of(&aid) {
                    self.remove_attached(&host, &aid);
                }
                self.set_face_down(&aid, false);
                self.add_to_zone(&aid, player, Zone::Exile);
                self.bump_action("attached_payment_exile", player);
            }
            self.add_to_zone(instance, player, Zone::Exile);
            if let Some(c) = ctx.as_mut() {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance)
                    .map_err(PlayError::ChoicePending)?;
            }
            self.current_cast_payments = None;
            return Ok(());
        }
        // Unified resolution path driven by `CastRouting`. Per-kind
        // behavior comes from the trait booleans; this body only knows
        // "board-placed vs non-board" and "attaches-to-target vs not."
        // Adding a new card kind = implement its `CastRouting` booleans
        // and (if needed) special-case any kind-specific bump or status.
        let is_board_placed = card_kind.is_board_placed();
        let attaches_to_target = card_kind.attaches_to_target();
        let payments = choices.hand_payment_ids.clone();

        // HAND payments (P.6 / C.10): attach to instance when BOARD-
        // placed, otherwise follow the spell-payment convention to
        // GRAVEYARD (no host to attach to).
        for hid in &payments {
            if is_board_placed {
                let _ = self.remove_from_zone_or_emit(
                    hid,
                    player,
                    Zone::Hand,
                    "play-hand-payment-attach",
                );
                self.add_attached(instance, hid);
                self.set_face_down(hid, true);
            } else {
                let _ = self.move_card_or_emit(
                    hid,
                    player,
                    Zone::Hand,
                    Zone::Graveyard,
                    "play-hand-payment-discard",
                );
            }
        }

        // ATTACHED-source payments (P.31): re-attach to instance when
        // BOARD-placed (face-down per P.17), else move to EXILE.
        for aid in choices.attached_payment_ids.clone() {
            if let Some(host) = self.host_of(&aid) {
                self.remove_attached(&host, &aid);
            }
            if is_board_placed {
                self.add_attached(instance, &aid);
                self.set_face_down(&aid, true);
                self.bump_action("attached_payment_transfer", player);
            } else {
                self.set_face_down(&aid, false);
                self.add_to_zone(&aid, player, Zone::Exile);
                self.bump_action("attached_payment_exile", player);
            }
        }

        // Place the cast card. P.33: it was removed from HAND at cast
        // time; resolution puts it in its destination here.
        if attaches_to_target {
            let target = choices
                .mutation_target
                .as_ref()
                .expect("validated by play_card");
            // P.26 / Z.7: a mutation fuses into the host's sleeve, it does
            // not attach as a strippable object. It therefore rides the
            // host's zone moves (P.29), is exempt from the P.8 cascade, and
            // does not count toward attached-count (C.16).
            if self.add_same_sleeve(target, instance) {
                self.set_face_down(instance, true);
                // Z.8 sleeve-as-atom conservation: the mutation card left
                // its own sleeve to share the host's. That vacated sleeve is
                // not destroyed — it attaches to the host as a cardless
                // sleeve (Z.6), strippable and counted by AttachedCount. The
                // id is derived from the mutation instance, which fuses
                // exactly once (P.33: it can't be recast once it leaves HAND).
                let shed = format!("{instance}:shed");
                self.mint_cardless_sleeve(&shed, player);
                self.add_attached(target, &shed);
                self.set_face_down(&shed, true);
            }
        } else if is_board_placed {
            self.add_to_zone(instance, player, Zone::Board);
            if card_kind.applies_summoning_sickness() {
                self.set_summoning_sick(instance, true); // B.3
            }
            if matches!(card_kind, CardType::Artifact) {
                self.bump_action("artifact_played", player);
            }
            if matches!(card_kind, CardType::Symbol) {
                // P.35: a Symbol cast has now resolved onto BOARD;
                // burn this player's per-turn cap. Failed casts never
                // reach here (the gate above returns SymbolCastCapReached
                // before any state mutation), so we don't ratchet the
                // flag for refused attempts.
                self.set_symbol_cast_this_turn(player, true);
                self.bump_action("symbol_played", player);
            }
        } else {
            // P.1 default destination: GRAVEYARD (spell convention,
            // typeless P.1, etc.). SelfExile rerouting is handled by
            // the early shortcut above and never reaches here.
            self.add_to_zone(instance, player, Zone::Graveyard);
        }

        // OnAttachedAsCost: BOARD-placed casts only. Fires per HAND
        // payment with the payment card as `self` and the host as
        // `partner`. Powers mantis-shrimp / zebra / pitch-synergy.
        if is_board_placed {
            for hid in &payments {
                if let Some(c) = ctx.as_mut() {
                    lua_api::fire_with_partner(
                        c.lua,
                        self,
                        c.oracle(),
                        EventName::OnAttachedAsCost,
                        hid,
                        instance,
                    )
                    .map_err(PlayError::ChoicePending)?;
                }
            }
        }

        // OnPlay: fires for every castable kind. The payment snapshot
        // was stashed on GameState by `play_card` before this resolver
        // ran, so handlers can read `game.payment_ids()` here.
        if let Some(c) = ctx.as_mut() {
            lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance)
                .map_err(PlayError::ChoicePending)?;
        }
        self.current_cast_payments = None;

        // OnEnterBoard: BOARD-placed casts only.
        if is_board_placed {
            if let Some(c) = ctx.as_mut() {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnEnterBoard, instance)
                    .map_err(PlayError::ChoicePending)?;
            }
        }

        Ok(())
    }

    /// B.8: any on-board creature with accumulated damage ≥ effective Y
    /// dies (Board → Graveyard). Mirrors the death-check inside
    /// `confirm_blocks` but reusable from non-combat damage paths
    /// (today: `game.damage(...)` from Lua handlers). Fires on_die
    /// handlers — NO, today this does NOT fire on_die, matching the
    /// existing combat path's behavior under ChoicePending (combat.rs
    /// ~line 497-501 TODO). When the Pending-through-on_die slice
    /// lands, both this path and the combat path should gain on_die
    /// firing in the same shape.
    ///
    /// Idempotent. Call after every damage application that targets a
    /// BOARD creature.
    /// B.8: every on-board creature whose accumulated damage has reached its
    /// effective toughness. Production path: `drain_deferred_events`
    /// consumes this and applies the hook-aware death sequence
    /// (12.3b OnWouldDie window). The historical eager sweep
    /// `cleanup_b8_damage_deaths` still exists for unit-test convenience
    /// (`#[cfg(test)]`) but no production code path calls it.
    pub fn damage_lethal_creatures(&self) -> Vec<InstanceId> {
        let mut to_kill: Vec<InstanceId> = Vec::new();
        for iid in self.a.board.iter().chain(self.b.board.iter()) {
            let is_creature = self
                .card_pool
                .get(iid)
                .map(|i| i.card().kind == crate::card::CardType::Creature)
                .unwrap_or(false);
            if !is_creature {
                continue;
            }
            let damage = self.card_pool.get(iid).map(|i| i.damage).unwrap_or(0.0);
            let y = self.effective_stats(iid).1;
            if damage > 0.0 && damage >= y {
                to_kill.push(iid.clone());
            }
        }
        to_kill
    }

    /// Eager B.8 sweep: kill every damage-lethal creature and cascade
    /// their attached lists to EXILE. Production-dead since 12.3b —
    /// real death resolution goes through `drain_deferred_events` +
    /// OnWouldDie. Kept for unit-test convenience where a test wants
    /// to exercise the pure "damage → move" step in isolation.
    #[cfg(test)]
    pub fn cleanup_b8_damage_deaths(&mut self) {
        let to_kill = self.damage_lethal_creatures();
        for iid in &to_kill {
            let owner = self
                .card_pool
                .get(iid)
                .map(|i| i.owner)
                .unwrap_or(self.active_player);
            let _ = self.move_card_or_emit(
                iid,
                owner,
                Zone::Board,
                Zone::Graveyard,
                "cleanup-b8-death",
            );
        }
    }

    /// C.15: any on-board creature with effective Y ≤ 0 dies (Board →
    /// Graveyard). Fires on_die handlers if `ctx` is provided. Idempotent
    /// — call after any state change that could lower a creature's
    /// effective stats (modifier add/remove, attached add/remove, static
    /// source movement). P.8 attached-cascade is still TODO (matches the
    /// gap in combat death).
    pub fn cleanup_zero_y_deaths(&mut self, ctx: Option<&mut EventContext>) {
        let on_board: Vec<InstanceId> = self
            .a
            .board
            .iter()
            .chain(self.b.board.iter())
            .cloned()
            .collect();
        let mut to_kill: Vec<InstanceId> = Vec::new();
        for iid in &on_board {
            let is_creature = self
                .card_pool
                .get(iid)
                .map(|i| i.card().kind == crate::card::CardType::Creature)
                .unwrap_or(false);
            if !is_creature {
                continue;
            }
            let y = self.effective_stats(iid).1;
            if y <= 0.0 {
                to_kill.push(iid.clone());
            }
        }
        // 12.3: route zero-Y deaths through the replacement chokepoint too,
        // so an OnWouldDie handler can shed-to-survive / redirect here as
        // well. The continuous check still swallows a ChoicePending (as
        // before) — the death is committed to state; a handler's pending
        // user choice can't suspend a continuous cleanup yet.
        // TODO(lua-yield): make this fn Result<(), ChoicePending> so death
        // triggers from continuous checks can suspend like in-play triggers.
        let _ = self.resolve_board_deaths(to_kill, ctx);
    }

    /// 12.3 death-replacement chokepoint. For each creature the caller has
    /// determined would die, fire `OnWouldDie` (self-only, if a ctx is
    /// present) BEFORE any move, then act on whatever the handler chose:
    ///   - `Prevent`  → clear accumulated damage (B.8) and leave it on the
    ///     BOARD; it did not die.
    ///   - `Redirect` → move BOARD→zone quietly — no on_die, no
    ///     `OnCreatureDies` broadcast, no P.8 cascade.
    ///   - none       → normal death: BOARD→GRAVEYARD, then on_die, the
    ///     watcher broadcast, and the P.8 attached-cascade.
    ///
    /// Returns the iids that actually died (reached the GRAVEYARD) so combat
    /// can record them. With `ctx = None` no handler runs and every creature
    /// dies normally — matching the behaviour of any ctx-less death path.
    pub fn resolve_board_deaths(
        &mut self,
        to_kill: Vec<InstanceId>,
        ctx: Option<&mut EventContext>,
    ) -> Result<Vec<InstanceId>, crate::choice::ChoicePending> {
        // Self-guard `settling_deaths` across the whole resolution: the
        // on_die / OnWouldDie fires below each drain, and that drain scans
        // for damage deaths — without the guard it would re-enter and
        // re-kill the very creature being resolved. Restored to the prior
        // value so nested resolutions compose; genuinely new deaths surface
        // at the caller's own settle loop (drain_deferred_events).
        let prev = self.settling_deaths;
        self.settling_deaths = true;
        let result = self.resolve_board_deaths_inner(to_kill, ctx);
        self.settling_deaths = prev;
        result
    }

    fn resolve_board_deaths_inner(
        &mut self,
        to_kill: Vec<InstanceId>,
        mut ctx: Option<&mut EventContext>,
    ) -> Result<Vec<InstanceId>, crate::choice::ChoicePending> {
        let mut died: Vec<InstanceId> = Vec::new();
        for iid in to_kill {
            let owner = self
                .card_pool
                .get(&iid)
                .map(|i| i.owner)
                .unwrap_or(self.active_player);

            // Fire OnWouldDie BEFORE any move; the handler may record a
            // replacement via game.prevent_death / game.redirect_death.
            self.pending_death_replacement = None;
            if let Some(c) = ctx.as_deref_mut() {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnWouldDie, &iid)?;
            }

            match self.pending_death_replacement.take() {
                Some(DeathReplacement::Prevent) => {
                    // Survive: reset the lethal damage so the next cleanup
                    // does not immediately re-kill it. Stays on the board.
                    self.set_damage(&iid, 0.0);
                }
                Some(DeathReplacement::Redirect(zone)) => {
                    // Quiet relocation — no on_die, no broadcast, no cascade.
                    let _ = self.move_card_or_emit(
                        &iid,
                        owner,
                        Zone::Board,
                        zone,
                        "would-die-redirect",
                    );
                }
                None => {
                    // Normal death: BOARD → GRAVEYARD, then triggers + cascade.
                    let _ = self.move_card_or_emit(
                        &iid,
                        owner,
                        Zone::Board,
                        Zone::Graveyard,
                        "death",
                    );
                    died.push(iid.clone());
                    if let Some(c) = ctx.as_deref_mut() {
                        lua_api::fire_self_only(
                            c.lua,
                            self,
                            c.oracle(),
                            EventName::OnDie,
                            &iid,
                        )?;
                        // OnCreatureDies broadcast to BOARD watchers (the
                        // dead card already left BOARD, so it's excluded).
                        let watchers: Vec<InstanceId> = self
                            .a
                            .board
                            .iter()
                            .chain(self.b.board.iter())
                            .cloned()
                            .collect();
                        for watcher in &watchers {
                            lua_api::fire_with_partner(
                                c.lua,
                                self,
                                c.oracle(),
                                EventName::OnCreatureDies,
                                watcher,
                                &iid,
                            )?;
                        }
                    }
                    // P.8: cascade attached → EXILE after on_die fires.
                    self.exile_remaining_attached(&iid);
                }
            }
        }
        Ok(died)
    }
}

#[cfg(test)]
#[path = "play_tests.rs"]
mod tests;
