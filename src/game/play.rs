//! Playing a card from hand: cost payment, destination, attachment.
//!
//! Mirrors RULES.md P.1, P.2, P.6, P.7, P.11, P.17.

mod activate;
mod errors;
mod payments;

use super::context::EventContext;
use super::lua_api;
use super::state::{GameState, InstanceId, PlayerId, StackItem, Zone};
use crate::card::{CardType, CostSource, EventName};
use crate::cast_routing::CastRouting;
use crate::choice::ResponseAction;
use std::collections::BTreeSet;

// Re-exports: PlayChoices/PlayError are referenced in this file;
// ActivateError isn't (it moved to play/activate.rs) but play_tests.rs
// imports it from this module path.
#[allow(unused_imports)]
pub use errors::ActivateError;
pub use errors::{PlayChoices, PlayError};

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

        if !card_kind.is_castable() {
            // TODO(types): Environment (→ BOARD per P.21 + P.22 slot management).
            return Err(PlayError::UnsupportedType(card_kind));
        }
        // Sorcery timing: a Spell with Timing::Sorcery cannot be cast while
        // a response window is open (main-phase only).
        let card_timing = inst_ref.card.timing;
        if card_timing == Some(crate::card::Timing::Sorcery) && self.priority.is_some() {
            return Err(PlayError::SorceryAtInstantSpeed);
        }

        // RULES P.32: declarative target category. If the card declares
        // a target category and no legal target exists, refuse the cast
        // before any state mutation. Counterspell uses `target = "chain"`
        // to refuse when the stack is empty.
        if let Some(target) = inst_ref.card.target {
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
            .map(|i| i.card.allow_x_zero)
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

        // P.12a + P.12b color-anchor on GRAVEYARD-source payments.
        // When the cast has a GRAVEYARD cost component and non-empty
        // colors, at least one card being exiled to pay it must share
        // a printed color with the cast (lenient — one anchor for the
        // whole bundle suffices). When the anchor is supplied, P.12b
        // suspends P.7a's identity check on HAND payments for this cast.
        //
        // Determines `gy_supplies_color_anchor`, used below to bypass
        // the HAND-identity checks (P.12b).
        let gy_supplies_color_anchor = if graveyard_needed > 0 {
            let cast_colors_set: BTreeSet<String> = cast_card_colors.iter().cloned().collect();
            if cast_colors_set.is_empty() {
                // Empty-color cast is a wildcard already; anchor moot.
                true
            } else {
                let pitch_ids: Vec<InstanceId> = if !choices.graveyard_payment_ids.is_empty() {
                    choices.graveyard_payment_ids.clone()
                } else {
                    let gy = &self.player(player).graveyard;
                    let start = gy.len().saturating_sub(graveyard_needed);
                    gy[start..].to_vec()
                };
                let mut found = false;
                for gid in &pitch_ids {
                    let pay_colors: BTreeSet<String> = self
                        .card_pool
                        .get(gid)
                        .map(|i| {
                            i.card
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
                    .map(|i| i.card.gy_hand_substitute)
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
                return Err(PlayError::NoHandPaymentForIdentity);
            }
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
            // Identity match: discard must share ≥1 color OR the
            // (non-empty) symbol with the casting card. Colorless +
            // no-symbol CASTS are wildcards (take any discard);
            // colorless + no-symbol discards are NOT — they must
            // still find identity overlap, which empty sets can't,
            // so they can't pay for any identified card.
            //
            // P.12b: when a color-matching GRAVEYARD pitch was supplied
            // for the same cast, P.7a is suspended — the anchor pitch
            // supplies the thematic alignment for the whole bundle.
            let cast_ident = self.card_identity(instance);
            if !cast_ident.is_empty() && !gy_supplies_color_anchor {
                let pay_ident = self.card_identity(hid);
                if cast_ident.is_disjoint(&pay_ident) {
                    return Err(PlayError::HandPaymentIdentityMismatch(hid.clone()));
                }
            }
            // C.14: a transparent card can only be attached to another
            // transparent card. For HAND payments to BOARD-placed casts
            // (P.6 attaches them), refuse when the payment is transparent
            // and the cast itself isn't. Transparent ↔ transparent is OK.
            if matches!(
                card_kind,
                CardType::Creature | CardType::Artifact | CardType::Environment
            ) && self.is_transparent(hid)
                && !self.is_transparent(instance)
            {
                return Err(PlayError::HandPaymentTransparentForBoardPlaced(
                    hid.clone(),
                ));
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
            let Some(target) = &choices.mutation_target else {
                return Err(PlayError::MutationTargetMissing);
            };
            let on_a = self.a.board.contains(target);
            let on_b = self.b.board.contains(target);
            let is_creature = self
                .card_pool
                .get(target)
                .map(|i| i.card.kind == CardType::Creature)
                .unwrap_or(false);
            if !(on_a || on_b) || !is_creature {
                return Err(PlayError::MutationTargetInvalid(target.clone()));
            }
            if self.has_restriction(target, crate::card::Restriction::CannotBeAttachedTo) {
                return Err(PlayError::MutationTargetInvalid(target.clone()));
            }
            // C.14: a transparent mutation can only attach to a
            // transparent target. Non-transparent mutations attach to
            // anything.
            if self.is_transparent(instance) && !self.is_transparent(target) {
                return Err(PlayError::MutationTargetInvalid(target.clone()));
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
                if inst.card.kind != required_kind {
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
            if !att_seen.insert(aid) {
                return Err(PlayError::DuplicateAttachedPayment(aid.clone()));
            }
            let host = self.host_of(aid);
            let valid = match host {
                Some(h) => {
                    self.player(player).board.contains(&h)
                        && self
                            .card_pool
                            .get(&h)
                            .map(|i| i.controller == player)
                            .unwrap_or(false)
                }
                None => false,
            };
            if !valid {
                return Err(PlayError::AttachedPaymentInvalid(aid.clone()));
            }
            // C.14: ATTACHED-source payments re-attach to BOARD-placed
            // casts (P.31). A transparent attached card can only land
            // on a transparent host.
            if matches!(
                card_kind,
                CardType::Creature | CardType::Artifact | CardType::Environment
            ) && self.is_transparent(aid)
                && !self.is_transparent(instance)
            {
                return Err(PlayError::AttachedPaymentInvalid(aid.clone()));
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
        };

        // MILL cost: top N of DECK → GRAVEYARD (P.11).
        for _ in 0..mill_needed {
            let Some(top) = self.player(player).deck.first().cloned() else {
                break;
            };
            payments_snapshot.mill.push(top.clone());
            let _ = self.move_card(&top, player, Zone::Deck, Zone::Graveyard);
        }

        // GRAVEYARD cost (P.12): if the caller supplied explicit
        // `graveyard_payment_ids`, exile those in order. Otherwise fall
        // back to the legacy "most-recent N" behavior (back of GY) —
        // preserves byte-identical semantics for callers that haven't
        // migrated to id-explicit GY payment yet.
        if choices.graveyard_payment_ids.is_empty() {
            for _ in 0..graveyard_needed {
                let Some(back) = self.player(player).graveyard.last().cloned() else {
                    break;
                };
                payments_snapshot.graveyard.push(back.clone());
                let _ = self.move_card(&back, player, Zone::Graveyard, Zone::Exile);
            }
        } else {
            payments_snapshot.graveyard = choices.graveyard_payment_ids.clone();
            for gid in choices.graveyard_payment_ids.clone() {
                let _ = self.move_card(&gid, player, Zone::Graveyard, Zone::Exile);
            }
        }

        // Stash the payment snapshot on state so OnPlay handlers can read
        // it via `game.payment_ids()`. resolve_played_card_inner clears
        // it after OnPlay fires. Survives the stack-resolve hop between
        // play_card (here) and resolve_played_card_inner.
        self.current_cast_payments = Some(payments_snapshot);

        // P.24: tap the substituting jewel as part of cost payment.
        if let Some(jewel_iid) = &choices.jewel_tap {
            self.set_tapped(jewel_iid, true);
            self.bump_action("jewel_tap_substitution", player);
        }

        // Clear View-style HAND-substitute payments: each chosen card
        // in GY moves GY → EXILE. Validation above confirmed eligibility.
        for gid in choices.gy_hand_payment_ids.clone() {
            let _ = self.move_card(&gid, player, Zone::Graveyard, Zone::Exile);
            self.bump_action("gy_hand_substitution", player);
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
                    );
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
        let _ = self.remove_from_zone(instance, player, Zone::Hand);

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
                                    .map(|inst| inst.card.id.clone())
                                    .unwrap_or_else(|| format!("?{card}"));
                                format!("[{i}] {:?}={}", controller, card_id)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                eprintln!(
                    "[CHAIN OVERFLOW] turn={} active={:?} chain_len={} contents:\n  {}",
                    self.turn,
                    self.active_player,
                    chain_depth,
                    chain_dump.join("\n  "),
                );
                self.bump_action("chain_overflow", self.active_player);
                super::bump_timeout_and_maybe_halt("drive_window_to_close (chain overflow)");
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
                                    .map(|i| i.card.id.clone())
                                    .unwrap_or_else(|| format!("?{card}"))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let failed_card_id = last_failed_card
                    .as_ref()
                    .and_then(|iid| self.card_pool.get(iid).map(|i| i.card.id.clone()))
                    .unwrap_or_else(|| "(unknown)".to_string());
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
                super::bump_timeout_and_maybe_halt("drive_window_to_close (response spin)");
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
            .map(|i| i.card.kind)
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
                i.card.cost.iter().any(|c| {
                    matches!(c.source, CostSource::SelfExile) && c.amount.max(0) > 0
                })
            })
            .unwrap_or(false);
        if self_exiles {
            for hid in choices.hand_payment_ids.clone() {
                let _ = self.move_card(&hid, player, Zone::Hand, Zone::Graveyard);
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
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance);
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
                let _ = self.remove_from_zone(hid, player, Zone::Hand);
                self.add_attached(instance, hid);
                self.set_face_down(hid, true);
            } else {
                let _ = self.move_card(hid, player, Zone::Hand, Zone::Graveyard);
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
            self.add_attached(target, instance);
            self.set_face_down(instance, true);
        } else if is_board_placed {
            self.add_to_zone(instance, player, Zone::Board);
            if card_kind.applies_summoning_sickness() {
                self.set_summoning_sick(instance, true); // B.3
            }
            if matches!(card_kind, CardType::Artifact) {
                self.bump_action("artifact_played", player);
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
                    );
                }
            }
        }

        // OnPlay: fires for every castable kind. The payment snapshot
        // was stashed on GameState by `play_card` before this resolver
        // ran, so handlers can read `game.payment_ids()` here.
        if let Some(c) = ctx.as_mut() {
            lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance);
        }
        self.current_cast_payments = None;

        // OnEnterBoard: BOARD-placed casts only.
        if is_board_placed {
            if let Some(c) = ctx.as_mut() {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnEnterBoard, instance);
            }
        }

        Ok(())
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
                .map(|i| i.card.kind == crate::card::CardType::Creature)
                .unwrap_or(false);
            if !is_creature {
                continue;
            }
            let y = self.effective_stats(iid).1;
            if y <= 0.0 {
                to_kill.push(iid.clone());
            }
        }
        let mut ctx = ctx;
        for iid in &to_kill {
            let owner = self
                .card_pool
                .get(iid)
                .map(|i| i.owner)
                .unwrap_or(self.active_player);
            let _ = self.move_card(iid, owner, Zone::Board, Zone::Graveyard);
            if let Some(c) = ctx.as_mut() {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnDie, iid);
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
                        iid,
                    );
                }
            }
            // P.8: cascade attached → EXILE after on_die fires.
            self.exile_remaining_attached(iid);
        }
    }



}

#[cfg(test)]
#[path = "play_tests.rs"]
mod tests;
