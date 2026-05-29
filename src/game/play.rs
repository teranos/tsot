//! Playing a card from hand: cost payment, destination, attachment.
//!
//! Mirrors RULES.md P.1, P.2, P.6, P.7, P.11, P.17.

use super::context::EventContext;
use super::lua_api;
use super::state::{GameState, InstanceId, PlayerId, Zone};
use crate::card::{CardType, CostSource, EventName};
use crate::choice::{ChoiceOracle, ChooseCardRequest};
use std::collections::BTreeSet;

/// Player-supplied choices when playing a card.
/// In this slice, only HAND payments require choice (which cards to spend).
/// MILL payments are deterministic (top N of DECK).
#[derive(Debug, Clone, Default)]
pub struct PlayChoices {
    /// One InstanceId per HAND cost-card the player chooses to spend.
    pub hand_payment_ids: Vec<InstanceId>,
    /// The value of X for variable-X cost components. Required if any cost
    /// component has `is_x: true`; the same X applies to every variable
    /// component on the card (per recast's `X hand + X graveyard` pattern).
    pub x_value: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayError {
    GameOver,
    NotInHand,
    /// This slice supports CREATURE and INSTANT card types.
    UnsupportedType(CardType),
    /// This slice supports HAND, MILL, and GRAVEYARD cost sources.
    UnsupportedCostSource(CostSource),
    /// GRAVEYARD doesn't have enough cards to pay the GRAVEYARD cost.
    InsufficientGraveyardForCost { needed: usize, have: usize },
    /// Card has a variable-X cost component but choices.x_value is None.
    VariableXValueMissing,
    /// HAND payment count must equal the card's total HAND cost.
    WrongHandPaymentCount { expected: usize, got: usize },
    /// A chosen HAND payment isn't in the player's hand, or is the card being played itself.
    HandPaymentInvalid(InstanceId),
    /// A HAND payment ID appears more than once in the choices.
    DuplicateHandPayment(InstanceId),
    /// DECK doesn't have enough cards to pay the MILL cost.
    InsufficientDeckForMill { needed: usize, have: usize },
}

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
        if self.winner.is_some() {
            return Err(PlayError::GameOver);
        }

        if !self.player(player).hand.contains(instance) {
            return Err(PlayError::NotInHand);
        }

        // Snapshot card data so the borrow on card_pool can be dropped.
        let inst_ref = self.card_pool.get(instance).ok_or(PlayError::NotInHand)?;
        let card_kind = inst_ref.card.kind;
        let card_cost = inst_ref.card.cost.clone();

        if !matches!(card_kind, CardType::Creature | CardType::Instant) {
            // TODO(types): handle Spell (→ GRAVEYARD per C.10), Artifact (→ BOARD per P.19),
            // Environment (→ BOARD per P.21 + P.22 slot management).
            return Err(PlayError::UnsupportedType(card_kind));
        }

        // Aggregate cost requirements per source.
        let mut hand_needed: usize = 0;
        let mut mill_needed: usize = 0;
        let mut graveyard_needed: usize = 0;
        // Variable-X: if any cost component has is_x, the player must have
        // pre-chosen X (via oracle.choose_int) and supplied it in choices.
        // The same X applies to every variable component.
        let has_variable_x = card_cost.iter().any(|c| c.is_x);
        let x_value = if has_variable_x {
            match choices.x_value {
                Some(v) => v.max(0) as usize,
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
                // TODO(costs): support SACRIFICE (P.16) and SELF (P.5).
                other => return Err(PlayError::UnsupportedCostSource(other)),
            }
        }

        if choices.hand_payment_ids.len() != hand_needed {
            return Err(PlayError::WrongHandPaymentCount {
                expected: hand_needed,
                got: choices.hand_payment_ids.len(),
            });
        }

        let mut seen: BTreeSet<&InstanceId> = BTreeSet::new();
        for hid in &choices.hand_payment_ids {
            if !seen.insert(hid) {
                return Err(PlayError::DuplicateHandPayment(hid.clone()));
            }
            if hid == instance {
                return Err(PlayError::HandPaymentInvalid(hid.clone()));
            }
            if !self.player(player).hand.contains(hid) {
                return Err(PlayError::HandPaymentInvalid(hid.clone()));
            }
        }

        let deck_have = self.player(player).deck.len();
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

        // All checks pass — apply mutations through journaled helpers.

        // MILL cost: top N of DECK → GRAVEYARD (P.11).
        for _ in 0..mill_needed {
            let Some(top) = self.player(player).deck.first().cloned() else {
                break;
            };
            let _ = self.move_card(&top, player, Zone::Deck, Zone::Graveyard);
        }

        // GRAVEYARD cost (P.12): most-recent N → EXILE. Deterministic
        // interpretation pending choice API; uses the back of the graveyard.
        for _ in 0..graveyard_needed {
            let Some(back) = self.player(player).graveyard.last().cloned() else {
                break;
            };
            let _ = self.move_card(&back, player, Zone::Graveyard, Zone::Exile);
        }

        // TODO(stack-phase-1): R.1.a — open a response window here, after
        // cost is paid but before resolution. Both players get priority;
        // resolution proceeds only after both pass. Until this is wired,
        // cards resolve immediately and instants can't be cast in response.
        //
        // Route to destination zone based on type. on_play fires after the
        // card and its payments have left hand and arrived at their final
        // locations — handlers observe the post-resolution state.
        let mut ctx = ctx;
        match card_kind {
            CardType::Creature => {
                // HAND payments leave hand and attach to the played card.
                for hid in choices.hand_payment_ids.clone() {
                    let _ = self.remove_from_zone(&hid, player, Zone::Hand);
                    self.add_attached(instance, &hid);
                    self.set_face_down(&hid, true);
                }
                // Played card → BOARD.
                let _ = self.move_card(instance, player, Zone::Hand, Zone::Board);
                self.set_summoning_sick(instance, true); // B.3

                if let Some(c) = ctx.as_mut() {
                    lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance);
                }
                if let Some(c) = ctx.as_mut() {
                    lua_api::fire_self_only(
                        c.lua,
                        self,
                        c.oracle(),
                        EventName::OnEnterBoard,
                        instance,
                    );
                }
            }
            CardType::Instant => {
                // P.1: instants resolve to GRAVEYARD. HAND payments follow
                // (no host to attach to).
                for hid in choices.hand_payment_ids.clone() {
                    let _ = self.move_card(&hid, player, Zone::Hand, Zone::Graveyard);
                }
                let _ = self.move_card(instance, player, Zone::Hand, Zone::Graveyard);

                if let Some(c) = ctx.as_mut() {
                    lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance);
                }
            }
            _ => unreachable!("validated above"),
        }

        Ok(())
    }

    /// Build a HAND payment vector by asking `oracle.choose_card` once per
    /// payment slot. Pool is `player.hand` minus the card being played and
    /// any cards already picked for this payment. Pure read of state; the
    /// oracle's recording captures each pick so a retry-on-suicide can flip
    /// individual payment slots without altering call sites.
    ///
    /// Fallback: if the oracle returns None (RandomOracle for empty pool, or
    /// a future oracle that declines), we pick the first remaining eligible
    /// card — payment is mandatory, so we can't skip a slot.
    pub fn resolve_hand_payment(
        &self,
        player: PlayerId,
        instance: &InstanceId,
        hand_needed: usize,
        oracle: &mut dyn ChoiceOracle,
    ) -> Vec<InstanceId> {
        let mut chosen: Vec<InstanceId> = Vec::with_capacity(hand_needed);
        let mut picked_set: BTreeSet<InstanceId> = BTreeSet::new();
        for slot in 0..hand_needed {
            let pool: Vec<InstanceId> = self
                .player(player)
                .hand
                .iter()
                .filter(|iid| *iid != instance && !picked_set.contains(*iid))
                .cloned()
                .collect();
            if pool.is_empty() {
                break;
            }
            let pool_for_fallback = pool.clone();
            let req = ChooseCardRequest {
                pool,
                optional: false,
                prompt: format!("hand payment slot {}", slot + 1),
            };
            let pick = oracle
                .choose_card(req)
                .unwrap_or_else(|| pool_for_fallback[0].clone());
            picked_set.insert(pick.clone());
            chosen.push(pick);
        }
        chosen
    }
}

#[cfg(test)]
#[path = "play_tests.rs"]
mod tests;
