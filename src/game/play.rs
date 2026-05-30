//! Playing a card from hand: cost payment, destination, attachment.
//!
//! Mirrors RULES.md P.1, P.2, P.6, P.7, P.11, P.17.

use super::context::EventContext;
use super::lua_api;
use super::state::{GameState, InstanceId, PlayerId, StackItem, Zone};
use crate::card::{CardType, CostSource, EventName};
use crate::choice::{ChoiceOracle, ChooseCardRequest, ResponseAction};
use std::collections::BTreeSet;

/// Player-supplied choices when playing a card.
/// In this slice, only HAND payments require choice (which cards to spend).
/// MILL payments are deterministic (top N of DECK).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayChoices {
    /// One InstanceId per HAND cost-card the player chooses to spend.
    pub hand_payment_ids: Vec<InstanceId>,
    /// The value of X for variable-X cost components. Required if any cost
    /// component has `is_x: true`; the same X applies to every variable
    /// component on the card (per recast's `X hand + X graveyard` pattern).
    pub x_value: Option<i32>,
    /// P.24: optionally tap one untapped jewel on the player's BOARD whose
    /// colors share at least one with the cast card, to substitute for one
    /// HAND-source cost component. Max one per cast. The substituted HAND
    /// count is reduced by 1 (so `hand_payment_ids.len()` should be the
    /// already-reduced count).
    pub jewel_tap: Option<InstanceId>,
    /// P.16: one InstanceId per SACRIFICE cost component. Each ID must be
    /// on the player's BOARD and they control it. Moves BOARD → GRAVEYARD
    /// as part of cost payment; on_die fires per sacrificed card.
    pub sacrifice_ids: Vec<InstanceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayError {
    GameOver,
    NotInHand,
    /// Card type not currently routable by `play_card`. Today: Creature,
    /// Spell, Artifact. Environment still unsupported.
    UnsupportedType(CardType),
    /// Spell (sorcery timing) cast while a response window is open. Per
    /// R.1 + sorcery convention: sorceries are main-phase only.
    SorceryAtInstantSpeed,
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
    /// P.24: jewel-tap substitution declared, but the chosen card isn't a
    /// valid jewel for this cast (not on player's BOARD, not untapped, not
    /// a jewel subtype, or color mismatch with cast card).
    InvalidJewelTap(InstanceId),
    /// P.24: jewel-tap declared on a card with no HAND-source cost component
    /// to substitute (would substitute nothing).
    JewelTapWithoutHandCost,
    /// Phase 3: a static restriction (e.g., flesh-eating-plant's
    /// `cannot_be_cost_paid`) forbids using this card as a HAND payment.
    HandPaymentForbidden(InstanceId),
    /// P.16: SACRIFICE payment count doesn't match the card's total
    /// SACRIFICE cost.
    WrongSacrificeCount { expected: usize, got: usize },
    /// P.16: a chosen sacrifice ID isn't on the player's BOARD, or the
    /// player doesn't control it.
    SacrificePaymentInvalid(InstanceId),
    /// P.16: a sacrifice ID appears more than once in the choices.
    DuplicateSacrifice(InstanceId),
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
        let mut ctx = ctx;
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

        if !matches!(
            card_kind,
            CardType::Creature | CardType::Spell | CardType::Artifact
        ) {
            // TODO(types): Environment (→ BOARD per P.21 + P.22 slot management).
            return Err(PlayError::UnsupportedType(card_kind));
        }
        // Sorcery timing: a Spell with Timing::Sorcery cannot be cast while
        // a response window is open (main-phase only).
        let card_timing = inst_ref.card.timing;
        if card_timing == Some(crate::card::Timing::Sorcery) && self.priority.is_some() {
            return Err(PlayError::SorceryAtInstantSpeed);
        }

        // Aggregate cost requirements per source.
        let mut hand_needed: usize = 0;
        let mut mill_needed: usize = 0;
        let mut graveyard_needed: usize = 0;
        let mut sacrifice_needed: usize = 0;
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
                CostSource::Sacrifice => sacrifice_needed += amount,
                // TODO(costs): support SELF (P.5).
                other => return Err(PlayError::UnsupportedCostSource(other)),
            }
        }

        // P.24: validate optional jewel-tap. Pull card colors once for both
        // the jewel-color check (here) and any future uses. After validation,
        // reduce `hand_needed` by 1 — the jewel substitutes for one HAND slot.
        let cast_card_colors: Vec<String> = self
            .card_pool
            .get(instance)
            .map(|i| {
                i.card
                    .colors
                    .iter()
                    .map(|c| c.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(jewel_iid) = &choices.jewel_tap {
            if hand_needed == 0 {
                return Err(PlayError::JewelTapWithoutHandCost);
            }
            let valid = self.is_valid_jewel_tap(player, jewel_iid, &cast_card_colors);
            if !valid {
                return Err(PlayError::InvalidJewelTap(jewel_iid.clone()));
            }
            hand_needed -= 1;
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
            // P.24/Phase 3: a static restriction can make a card unpayable
            // as a HAND cost (flesh-eating-plant on opponent insects).
            if self.has_restriction(hid, crate::card::Restriction::CannotBeCostPaid) {
                return Err(PlayError::HandPaymentForbidden(hid.clone()));
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
                if inst.card.kind != required_kind {
                    return Err(PlayError::SacrificePaymentInvalid(sid.clone()));
                }
            }
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

        // P.24: tap the substituting jewel as part of cost payment.
        if let Some(jewel_iid) = &choices.jewel_tap {
            self.set_tapped(jewel_iid, true);
            self.bump_action("jewel_tap_substitution", player);
        }

        // P.16: SACRIFICE — move chosen BOARD cards to GRAVEYARD and fire
        // on_die per card (matches combat's death-detection sequence).
        let sac_ids: Vec<InstanceId> = choices.sacrifice_ids.clone();
        for sid in &sac_ids {
            let _ = self.move_card(sid, player, Zone::Board, Zone::Graveyard);
            self.bump_action("sacrificed_as_cost", player);
        }
        if let Some(c) = ctx.as_mut() {
            for sid in &sac_ids {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnDie, sid);
            }
        }

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
        while self.priority.is_some() {
            let next = self.priority.as_ref().expect("checked is_some").next_to_act;
            let action = match ctx.as_mut() {
                Some(c) => c.oracle().respond_or_pass(self, next),
                None => ResponseAction::Pass,
            };
            match action {
                ResponseAction::Respond { card, choices } => {
                    self.bump_action("instant_response_played", next);
                    let _ = self.play_card(next, &card, choices, ctx.as_deref_mut());
                }
                ResponseAction::Pass => match self.pass_priority() {
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
                },
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
            .map(|i| i.card.kind)
            .unwrap_or(CardType::Unspecified);
        let mut ctx = ctx;
        match card_kind {
            CardType::Creature => {
                let payments = choices.hand_payment_ids.clone();
                for hid in &payments {
                    let _ = self.remove_from_zone(hid, player, Zone::Hand);
                    self.add_attached(instance, hid);
                    self.set_face_down(hid, true);
                }
                let _ = self.move_card(instance, player, Zone::Hand, Zone::Board);
                self.set_summoning_sick(instance, true); // B.3

                // Fire OnAttachedAsCost on each payment card BEFORE on_play.
                // The handler sees the attached card as `self` and the host
                // (the played card) as `partner`. Powers mantis-shrimp /
                // zebra / future pitch-synergy cantrips.
                for hid in &payments {
                    if let Some(c) = ctx.as_mut() {
                        lua_api::fire_with_partner(
                            c.lua,
                            self,
                            c.oracle(),
                            EventName::OnAttachedAsCost,
                            hid,
                            instance,
                        );
                    }
                }
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
            CardType::Artifact => {
                // P.19: artifact → BOARD. HAND payments attach (P.6), same
                // pattern as creature except: artifacts don't get summoning
                // sickness (B.3 is creature-specific; artifacts don't attack).
                // on_play + on_enter_board fire so artifact statics participate
                // in the standard ETB flow.
                let payments = choices.hand_payment_ids.clone();
                for hid in &payments {
                    let _ = self.remove_from_zone(hid, player, Zone::Hand);
                    self.add_attached(instance, hid);
                    self.set_face_down(hid, true);
                }
                let _ = self.move_card(instance, player, Zone::Hand, Zone::Board);
                self.bump_action("artifact_played", player);

                for hid in &payments {
                    if let Some(c) = ctx.as_mut() {
                        lua_api::fire_with_partner(
                            c.lua,
                            self,
                            c.oracle(),
                            EventName::OnAttachedAsCost,
                            hid,
                            instance,
                        );
                    }
                }
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
            CardType::Spell => {
                // P.1 + C.10: spells resolve to GRAVEYARD. HAND payments
                // follow (no host to attach to). Instant vs sorcery only
                // affects cast timing (enforced in `play_card` validation);
                // resolution is the same either way.
                for hid in choices.hand_payment_ids.clone() {
                    let _ = self.move_card(&hid, player, Zone::Hand, Zone::Graveyard);
                }
                let _ = self.move_card(instance, player, Zone::Hand, Zone::Graveyard);

                if let Some(c) = ctx.as_mut() {
                    lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance);
                }
            }
            _ => unreachable!("validated by play_card"),
        }
        Ok(())
    }

    /// Build a HAND payment vector by asking `oracle.choose_card` once per
    /// P.24: returns true iff `tap_iid` is an untapped jewel OR crystal on
    /// `player`'s BOARD whose color source intersects `cast_colors`.
    ///
    /// Color source differs by subtype:
    /// - `jewel` matches by the jewel's own printed colors.
    /// - `crystal` matches by the colors of cards ATTACHED to the crystal
    ///   (since crystals print with all colors, matching their own would
    ///   be trivial — the attached cards carry the meaningful constraint).
    pub fn is_valid_jewel_tap(
        &self,
        player: PlayerId,
        tap_iid: &InstanceId,
        cast_colors: &[String],
    ) -> bool {
        if !self.player(player).board.contains(tap_iid) {
            return false;
        }
        let Some(tap_card) = self.card_pool.get(tap_iid) else {
            return false;
        };
        if tap_card.tapped {
            return false;
        }
        if tap_card.controller != player {
            return false;
        }
        if cast_colors.is_empty() {
            return false;
        }
        let is_jewel = tap_card
            .card
            .subtypes
            .iter()
            .any(|s| s.eq_ignore_ascii_case("jewel"));
        let is_crystal = tap_card
            .card
            .subtypes
            .iter()
            .any(|s| s.eq_ignore_ascii_case("crystal"));
        if is_jewel {
            return tap_card.card.colors.iter().any(|c| {
                let lc = c.to_ascii_lowercase();
                cast_colors.contains(&lc)
            });
        }
        if is_crystal {
            // Match against colors of attached cards.
            for att_iid in &tap_card.attached {
                if let Some(att) = self.card_pool.get(att_iid) {
                    for col in &att.card.colors {
                        let lc = col.to_ascii_lowercase();
                        if cast_colors.contains(&lc) {
                            return true;
                        }
                    }
                }
            }
            return false;
        }
        false
    }

    /// First untapped same-color jewel on `player`'s BOARD that's a valid
    /// jewel-tap substitute for casting `cast_iid` (which must be in hand
    /// or otherwise have known colors via card_pool). Returns None if no
    /// such jewel exists. Used by the sim AI to opportunistically prefer
    /// jewel-tap over pitching a hand card.
    pub fn find_jewel_tap_candidate(
        &self,
        player: PlayerId,
        cast_iid: &InstanceId,
    ) -> Option<InstanceId> {
        let cast_colors: Vec<String> = self
            .card_pool
            .get(cast_iid)
            .map(|i| {
                i.card
                    .colors
                    .iter()
                    .map(|c| c.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        if cast_colors.is_empty() {
            return None;
        }
        self.player(player)
            .board
            .iter()
            .find(|iid| self.is_valid_jewel_tap(player, iid, &cast_colors))
            .cloned()
    }

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
                // Phase 3: filter out cards with a `cannot_be_cost_paid`
                // restriction so the oracle never sees them as candidates.
                .filter(|iid| {
                    !self.has_restriction(iid, crate::card::Restriction::CannotBeCostPaid)
                })
                .cloned()
                .collect();
            if pool.is_empty() {
                break;
            }
            let pool_for_fallback = pool.clone();
            // Hand-payment pool is entirely the player's own hand — the
            // Pass asker + host so the oracle can score candidates via the
            // pitch-score heuristic (pitch-payoff cards preferred when the
            // host color matches; jewels / mantis-shrimp / zebra benefit).
            let req = ChooseCardRequest {
                pool,
                asker: Some(player),
                host: Some(instance.clone()),
                optional: false,
                prompt: format!("hand payment slot {}", slot + 1),
            };
            let pick = oracle
                .choose_card(self, req)
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
