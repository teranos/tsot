//! Playing a card from hand: cost payment, destination, attachment.
//!
//! Mirrors RULES.md P.1, P.2, P.6, P.7, P.11, P.17.

use super::context::EventContext;
use super::lua_api;
use super::state::{GameState, InstanceId, PlayerId, StackItem, Zone};
use crate::card::{CardType, CostSource, EventName};
use crate::choice::{ChoiceOracle, ChooseCardRequest, ResponseAction};
use std::collections::BTreeSet;

/// Outcomes for `activate_ability`. The sim AI is expected to call only
/// when validation will pass (cheap pre-checks), but the engine still
/// enforces each rule so manual call sites and replays stay honest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivateError {
    /// Source iid not in the card pool.
    SourceMissing,
    /// `ability_idx` out of range for this card's `activated` array.
    NoSuchAbility,
    /// Source is not in the controller's BOARD zone. Activations from
    /// other zones (hand, graveyard, attached) are a v2 extension.
    NotOnBoard,
    /// Tap cost: source is already tapped.
    AlreadyTapped,
    /// Tap cost: source is a creature with B.3 summoning sickness and
    /// no haste.
    SummoningSick,
    /// One of the cost components cannot be paid from controller state
    /// (insufficient hand size, deck depth, graveyard size, etc.) or
    /// the cost source isn't supported by this v1 activation path
    /// (Sacrifice / SelfExile pending).
    CannotPayComponents,
    /// The ability's optional `validate` hook returned false (or
    /// errored). No cost is paid in this case — the hook's purpose is
    /// to refuse activation when no legal target exists, so the AI
    /// doesn't burn cards on a no-op.
    NoLegalTarget,
    /// RULES P.30: X < 1 on an X-cost activation that doesn't opt
    /// into X = 0 (`Card.allow_x_zero = false`).
    XBelowMinimum,
}

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
    /// MUTATION target: required when the cast card has `kind = Mutation`.
    /// Names the on-board creature the mutation will attach to. Any
    /// creature is a legal target (friendly or opposing).
    pub mutation_target: Option<InstanceId>,
    /// Clear View-style HAND-payment substitutes drawn from the
    /// controller's GRAVEYARD. Each iid must be in the controller's
    /// graveyard and have `Card.gy_hand_substitute = true`. Each one
    /// fills one HAND-source slot of the cast and moves GY → EXILE
    /// during cost payment. Does NOT satisfy P.7a identity for the
    /// cast — only the `hand_payment_ids` slots are identity-checked,
    /// so casts of identity-bearing spells still need at least one
    /// matching card in hand for each non-substituted slot.
    #[serde(default)]
    pub gy_hand_payment_ids: Vec<InstanceId>,
    /// P.31: one InstanceId per ATTACHED-source cost slot. Each id must
    /// currently be attached to a card the player controls on the BOARD.
    /// On resolution the cards detach and either re-attach to the played
    /// card (if BOARD-placed) or move to EXILE (non-BOARD).
    #[serde(default)]
    pub attached_payment_ids: Vec<InstanceId>,
    /// P.12 + P.12a: explicit choice of which GY cards to exile to pay
    /// `N graveyard` cost components. When non-empty, must contain
    /// exactly `graveyard_needed` ids and each must be in the player's
    /// GRAVEYARD; the engine exiles them in the provided order. When
    /// empty (the legacy path), the engine falls back to exiling the
    /// most-recent N cards from the back of the GY. The empty fallback
    /// keeps the slice's existing behavior byte-identical for callers
    /// that haven't migrated yet; P.12a's color-anchor rule (added in a
    /// follow-up slice) needs explicit ids to enforce.
    #[serde(default)]
    pub graveyard_payment_ids: Vec<InstanceId>,
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
    /// P.12: explicit `graveyard_payment_ids` was non-empty but its
    /// length doesn't match the card's total GRAVEYARD cost. The empty
    /// case falls back to the legacy "back of GY" behavior and is not
    /// a count error.
    WrongGraveyardPaymentCount { expected: usize, got: usize },
    /// P.12: a chosen GRAVEYARD-payment id isn't in the player's
    /// GRAVEYARD (or doesn't exist in the card pool).
    GraveyardPaymentInvalid(InstanceId),
    /// P.12: a GRAVEYARD-payment id appears more than once.
    DuplicateGraveyardPayment(InstanceId),
    /// P.12a: cast has non-empty colors and a GRAVEYARD-source cost
    /// component, but none of the cards being exiled (either the
    /// explicit `graveyard_payment_ids` or the legacy back-of-GY) share
    /// a printed color with the cast. The color-anchor requirement is
    /// lenient: a single color-matching pitch anywhere in the bundle
    /// satisfies it.
    NoGraveyardPaymentForColor,
    /// Card has a variable-X cost component but choices.x_value is None.
    VariableXValueMissing,
    /// RULES P.30: X < 1 on a card that doesn't opt into X = 0
    /// (`Card.allow_x_zero = false`).
    XBelowMinimum,
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
    /// HAND payment doesn't share an identity element (color or
    /// symbol) with the casting card. Cards with no colors and no
    /// symbol act as wildcards on either side — this only fires when
    /// both have non-empty identity sets that don't intersect.
    HandPaymentIdentityMismatch(InstanceId),
    /// MUTATION cast missing a target creature.
    MutationTargetMissing,
    /// MUTATION target isn't a creature on either BOARD.
    MutationTargetInvalid(InstanceId),
    /// P.16: SACRIFICE payment count doesn't match the card's total
    /// SACRIFICE cost.
    WrongSacrificeCount { expected: usize, got: usize },
    /// P.16: a chosen sacrifice ID isn't on the player's BOARD, or the
    /// player doesn't control it.
    SacrificePaymentInvalid(InstanceId),
    /// P.16: a sacrifice ID appears more than once in the choices.
    DuplicateSacrifice(InstanceId),
    /// A GY-hand-substitute payment isn't in the player's graveyard.
    GyHandSubstituteNotInGraveyard(InstanceId),
    /// A GY-hand-substitute payment doesn't have
    /// `Card.gy_hand_substitute = true` — only Clear View-style cards
    /// qualify today.
    GyHandSubstituteNotEligible(InstanceId),
    /// Same iid appears twice in `gy_hand_payment_ids`.
    DuplicateGyHandSubstitute(InstanceId),
    /// `gy_hand_payment_ids` declared on a card with no HAND-source
    /// cost component to substitute (would substitute nothing).
    GyHandSubstituteWithoutHandCost,
    /// P.31: ATTACHED payment count doesn't match the card's total
    /// ATTACHED cost.
    WrongAttachedPaymentCount { expected: usize, got: usize },
    /// P.31: a chosen attached id isn't attached to a card the player
    /// controls on the BOARD.
    AttachedPaymentInvalid(InstanceId),
    /// P.31: an attached payment id appears more than once.
    DuplicateAttachedPayment(InstanceId),
    /// The card's optional `validate` hook returned false at cast time —
    /// typically "no legal target exists for this card." No cost is paid
    /// (the check runs before any state mutation). Parallel to
    /// `ActivateError::NoLegalTarget` but for cast.
    CastValidateFailed,
    /// C.14: a transparent card cannot be a HAND-source payment for a
    /// card placed on the BOARD when played.
    HandPaymentTransparentForBoardPlaced(InstanceId),
    /// All HAND slots were filled by GY substitutes on a cast that
    /// requires identity matching (cast has non-empty colors or
    /// symbols). Clear View doesn't carry identity, so at least one
    /// HAND payment from hand is required when the cast has any
    /// identity at all. A 1-hand blue cast can't be paid solely by
    /// Clear View — there's no hand-payment slot left to satisfy
    /// P.7a's identity check.
    NoHandPaymentForIdentity,
}

impl GameState {
    /// Card identity for HAND-cost matching per RULES P.7a: the set of
    /// lowercase colors plus every non-empty `symbol` on the card. A
    /// card with no colors and no symbols returns an empty set — empty
    /// identity is a wildcard *when being cast* (any payment matches it)
    /// and a non-match *when being paid* (empty intersects nothing).
    pub fn card_identity(&self, iid: &InstanceId) -> BTreeSet<String> {
        let mut ident = BTreeSet::new();
        for color in self.effective_colors(iid) {
            ident.insert(color);
        }
        if let Some(inst) = self.card_pool.get(iid) {
            for sym in &inst.card.symbols {
                if !sym.is_empty() {
                    ident.insert(sym.clone());
                }
            }
        }
        ident
    }

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
            CardType::Creature
                | CardType::Spell
                | CardType::Artifact
                | CardType::Mutation
                | CardType::Unspecified
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
            // C.14: transparent cards can't be HAND payment for BOARD-
            // placed casts (P.6 attaches HAND payments; transparent can't
            // be attached).
            if matches!(
                card_kind,
                CardType::Creature | CardType::Artifact | CardType::Environment
            ) {
                let is_transparent = self
                    .card_pool
                    .get(hid)
                    .map(|i| {
                        i.card
                            .colors
                            .iter()
                            .any(|c| c.eq_ignore_ascii_case("transparent"))
                    })
                    .unwrap_or(false);
                if is_transparent {
                    return Err(PlayError::HandPaymentTransparentForBoardPlaced(
                        hid.clone(),
                    ));
                }
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
        // on either BOARD to attach to. Any creature qualifies.
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
        }

        // All checks pass — apply mutations through journaled helpers.

        // MILL cost: top N of DECK → GRAVEYARD (P.11).
        for _ in 0..mill_needed {
            let Some(top) = self.player(player).deck.first().cloned() else {
                break;
            };
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
                let _ = self.move_card(&back, player, Zone::Graveyard, Zone::Exile);
            }
        } else {
            for gid in choices.graveyard_payment_ids.clone() {
                let _ = self.move_card(&gid, player, Zone::Graveyard, Zone::Exile);
            }
        }

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
            }
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
            return Ok(());
        }
        match card_kind {
            CardType::Creature => {
                let payments = choices.hand_payment_ids.clone();
                for hid in &payments {
                    let _ = self.remove_from_zone(hid, player, Zone::Hand);
                    self.add_attached(instance, hid);
                    self.set_face_down(hid, true);
                }
                // P.31 BOARD-placed branch: detach attached payments from
                // their current hosts and re-attach to the new instance.
                for aid in choices.attached_payment_ids.clone() {
                    if let Some(host) = self.host_of(&aid) {
                        self.remove_attached(&host, &aid);
                    }
                    self.add_attached(instance, &aid);
                    self.set_face_down(&aid, true);
                    self.bump_action("attached_payment_transfer", player);
                }
                // P.33: cast card was removed from HAND at cast time;
                // resolution places it directly onto BOARD here.
                self.add_to_zone(instance, player, Zone::Board);
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
                // P.31 BOARD-placed branch.
                for aid in choices.attached_payment_ids.clone() {
                    if let Some(host) = self.host_of(&aid) {
                        self.remove_attached(&host, &aid);
                    }
                    self.add_attached(instance, &aid);
                    self.set_face_down(&aid, true);
                    self.bump_action("attached_payment_transfer", player);
                }
                // P.33: cast card was removed from HAND at cast time.
                self.add_to_zone(instance, player, Zone::Board);
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
                // P.31 non-BOARD branch: attached payments → EXILE.
                for aid in choices.attached_payment_ids.clone() {
                    if let Some(host) = self.host_of(&aid) {
                        self.remove_attached(&host, &aid);
                    }
                    self.set_face_down(&aid, false);
                    self.add_to_zone(&aid, player, Zone::Exile);
                    self.bump_action("attached_payment_exile", player);
                }
                // P.33: cast card was removed from HAND at cast time;
                // resolution places it directly into GRAVEYARD here (C.10).
                self.add_to_zone(instance, player, Zone::Graveyard);

                if let Some(c) = ctx.as_mut() {
                    lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance);
                }
            }
            CardType::Mutation => {
                // Mutation: HAND payments → GRAVEYARD (like spells, no
                // host accrual on the mutation itself). The mutation card
                // leaves HAND and ATTACHES to the chosen target creature
                // via add_attached + face-down per P.17. Its on-board
                // static effects fire from the attached position (the
                // static system already iterates attached cards as sources).
                for hid in choices.hand_payment_ids.clone() {
                    let _ = self.move_card(&hid, player, Zone::Hand, Zone::Graveyard);
                }
                // P.31 non-BOARD branch (mutations don't occupy a BOARD
                // slot per P.26, so attached payments → EXILE).
                for aid in choices.attached_payment_ids.clone() {
                    if let Some(host) = self.host_of(&aid) {
                        self.remove_attached(&host, &aid);
                    }
                    self.set_face_down(&aid, false);
                    self.add_to_zone(&aid, player, Zone::Exile);
                    self.bump_action("attached_payment_exile", player);
                }
                let target = choices
                    .mutation_target
                    .as_ref()
                    .expect("validated by play_card");
                // P.33: cast card was removed from HAND at cast time;
                // resolution attaches it directly to the host here.
                self.add_attached(target, instance);
                self.set_face_down(instance, true);

                if let Some(c) = ctx.as_mut() {
                    lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance);
                }
            }
            CardType::Unspecified => {
                // P.1: a card with no declared type resolves to GRAVEYARD
                // (the default destination). HAND payments follow the
                // spell-payment convention; there's no host to attach to.
                // ATTACHED payments take the non-BOARD branch (EXILE).
                // SelfExile-cost typeless cards are handled by the earlier
                // shortcut and never reach this arm.
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
                self.add_to_zone(instance, player, Zone::Graveyard);
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
            return self.effective_colors(tap_iid).iter().any(|c| cast_colors.contains(c));
        }
        if is_crystal {
            // Match against colors of attached cards (effective, so any
            // static-granted glow / color from a mutation on an attached
            // card also counts).
            for att_iid in &tap_card.attached {
                for col in self.effective_colors(att_iid) {
                    if cast_colors.contains(&col) {
                        return true;
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
        let cast_colors: Vec<String> = self.effective_colors(cast_iid);
        if cast_colors.is_empty() {
            return None;
        }
        self.player(player)
            .board
            .iter()
            .find(|iid| self.is_valid_jewel_tap(player, iid, &cast_colors))
            .cloned()
    }

    /// Count cards in `player`'s hand whose identity intersects the
    /// cast card's identity per P.7a. Used by the sim AI to decide
    /// whether Clear View substitutes are needed to cover slots the
    /// hand can't fill with identity-matching cards.
    pub fn identity_matching_hand_count(
        &self,
        player: PlayerId,
        cast_iid: &InstanceId,
    ) -> usize {
        let cast_ident = self.card_identity(cast_iid);
        // C.14: transparent cards can't pay for BOARD-placed casts.
        let cast_is_board_placed = self
            .card_pool
            .get(cast_iid)
            .map(|i| {
                matches!(
                    i.card.kind,
                    crate::card::CardType::Creature
                        | crate::card::CardType::Artifact
                        | crate::card::CardType::Environment
                )
            })
            .unwrap_or(false);
        let is_transparent = |h: &InstanceId| -> bool {
            self.card_pool
                .get(h)
                .map(|i| {
                    i.card
                        .colors
                        .iter()
                        .any(|c| c.eq_ignore_ascii_case("transparent"))
                })
                .unwrap_or(false)
        };
        if cast_ident.is_empty() {
            // Wildcard cast — every non-transparent hand card matches
            // (transparent excluded when cast is board-placed per C.14).
            return self
                .player(player)
                .hand
                .iter()
                .filter(|h| *h != cast_iid)
                .filter(|h| !cast_is_board_placed || !is_transparent(h))
                .count();
        }
        self.player(player)
            .hand
            .iter()
            .filter(|h| *h != cast_iid)
            .filter(|h| !cast_is_board_placed || !is_transparent(h))
            .filter(|h| {
                let pay_ident = self.card_identity(h);
                !cast_ident.is_disjoint(&pay_ident)
            })
            .count()
    }

    /// Pick up to `max_count` Clear View-style GY-substitute cards
    /// from `player`'s graveyard, in graveyard order. Each returned
    /// iid is a card with `Card.gy_hand_substitute = true` and lives
    /// in the controller's GRAVEYARD. The sim AI uses these to fill
    /// HAND slots that the hand's identity-matching cards can't cover.
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
            if y <= 0 {
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
            }
        }
    }

    /// P.31: collect up to `max_count` attached iids from cards the player
    /// controls on the BOARD. Iteration order: board iteration order, then
    /// per-host attached order. No scoring — sim uses first-N selection.
    pub fn find_attached_payments(
        &self,
        player: PlayerId,
        max_count: usize,
    ) -> Vec<InstanceId> {
        let mut out = Vec::new();
        for host_iid in &self.player(player).board {
            let Some(host) = self.card_pool.get(host_iid) else { continue };
            for aid in &host.attached {
                if out.len() >= max_count {
                    return out;
                }
                out.push(aid.clone());
            }
        }
        out
    }

    /// Sim AI helper: pick `n` GY cards to pay an `N graveyard` cost on
    /// a cast, prioritizing P.12a's color-anchor requirement. Returns up
    /// to `n` ids:
    ///
    /// - If the cast has non-empty colors, the first slot (if possible)
    ///   is filled with a color-matching GY card so P.12a is satisfied.
    /// - Remaining slots are filled deterministically from the front of
    ///   the GY, skipping any id already chosen.
    /// - When no color-matching card exists in GY but the cast has
    ///   colors, the returned bundle won't anchor — the engine will
    ///   reject the cast with `NoGraveyardPaymentForColor`. That's the
    ///   intended signal back to the AI's existing failed-cast retry.
    pub fn resolve_graveyard_payment(
        &self,
        player: PlayerId,
        cast_iid: &InstanceId,
        n: usize,
    ) -> Vec<InstanceId> {
        if n == 0 {
            return Vec::new();
        }
        let cast_colors: BTreeSet<String> = self
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
        let gy = &self.player(player).graveyard;
        let mut picked: Vec<InstanceId> = Vec::with_capacity(n);
        if !cast_colors.is_empty() {
            for iid in gy {
                let pay_colors: BTreeSet<String> = self
                    .card_pool
                    .get(iid)
                    .map(|i| {
                        i.card
                            .colors
                            .iter()
                            .map(|c| c.to_ascii_lowercase())
                            .collect()
                    })
                    .unwrap_or_default();
                if cast_colors.iter().any(|c| pay_colors.contains(c)) {
                    picked.push(iid.clone());
                    break;
                }
            }
        }
        for iid in gy {
            if picked.len() >= n {
                break;
            }
            if !picked.contains(iid) {
                picked.push(iid.clone());
            }
        }
        picked
    }

    pub fn find_gy_hand_substitutes(
        &self,
        player: PlayerId,
        _cast_iid: &InstanceId,
        max_count: usize,
    ) -> Vec<InstanceId> {
        self.player(player)
            .graveyard
            .iter()
            .filter(|iid| {
                self.card_pool
                    .get(*iid)
                    .map(|i| i.card.gy_hand_substitute)
                    .unwrap_or(false)
            })
            .take(max_count)
            .cloned()
            .collect()
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
        let cast_ident = self.card_identity(instance);
        let identity_matches = |hid: &InstanceId| -> bool {
            if cast_ident.is_empty() {
                return true;
            }
            let pay_ident = self.card_identity(hid);
            !cast_ident.is_disjoint(&pay_ident)
        };

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
                // Identity-match: discard must share a color or
                // symbol with the casting card, or be a no-identity
                // wildcard (no colors and no symbol).
                .filter(|iid| identity_matches(iid))
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

    /// Fire the activated ability at index `ability_idx` on `iid`.
    /// Per RULES A.5: pays the cost, then resolves the effect inline —
    /// no stack, no response window. Caller validates eligibility via
    /// `can_activate` before calling; this method re-validates and
    /// returns an `ActivateError` if the call slipped through stale.
    pub fn activate_ability(
        &mut self,
        iid: &InstanceId,
        ability_idx: usize,
        x_value: Option<i32>,
        mut ctx: Option<&mut EventContext>,
    ) -> Result<(), ActivateError> {
        // First pass: read everything we need from the card_pool entry
        // and from any static-granted activation at this index. Then
        // release the borrows. All subsequent steps may mutate self
        // (set_tapped, smart-discard, fire_validate, etc.) — they
        // can't coexist with immutable borrows on inst/ability.
        let (
            controller,
            is_creature,
            inst_tapped,
            inst_summoning_sick,
            cost_tap,
            components,
            handler,
            validate,
            ability_target,
            allow_x_zero,
        ) = {
            let inst = self
                .card_pool
                .get(iid)
                .ok_or(ActivateError::SourceMissing)?;
            // Index walks printed activations first, then static-granted
            // ones via activation_at. Both paths share the same shape.
            let ability = self
                .activation_at(iid, ability_idx)
                .ok_or(ActivateError::NoSuchAbility)?;
            (
                inst.controller,
                inst.card.kind == CardType::Creature,
                inst.tapped,
                inst.summoning_sick,
                ability.cost_tap,
                ability.cost_components.clone(),
                ability.effect.clone(),
                ability.validate.clone(),
                ability.target,
                inst.card.allow_x_zero,
            )
        };
        // RULES A.9 + P.32: declarative target category. If set and no
        // legal target exists, refuse activation before any cost.
        if let Some(target) = ability_target {
            if !self.is_target_legal(target) {
                return Err(ActivateError::NoLegalTarget);
            }
        }

        // Source must be on its controller's BOARD. v1 doesn't model
        // activations from hand / graveyard / attached.
        if !self.player(controller).board.contains(iid) {
            return Err(ActivateError::NotOnBoard);
        }

        // Tap-cost gate.
        if cost_tap {
            if inst_tapped {
                return Err(ActivateError::AlreadyTapped);
            }
            if is_creature && inst_summoning_sick && !self.has_keyword(iid, "haste") {
                return Err(ActivateError::SummoningSick);
            }
        }

        // Component-cost gate. Variable-X components (`is_x = true`)
        // multiply by x_value; the caller is required to provide a
        // value if any component uses X. Pre-validate every component
        // is payable from the controller's current zones. Once we
        // pass this, the payment loop below cannot fail half-way.
        let has_x = components.iter().any(|c| c.is_x);
        if has_x && x_value.is_none() {
            return Err(ActivateError::CannotPayComponents);
        }
        // RULES P.30: minimum X = 1 unless the card opts into X = 0.
        if has_x {
            if let Some(v) = x_value {
                if v < 1 && !allow_x_zero {
                    return Err(ActivateError::XBelowMinimum);
                }
            }
        }
        let x_val = x_value.unwrap_or(0).max(0);
        let mut hand_need = 0usize;
        let mut mill_need = 0usize;
        let mut gy_need = 0usize;
        for c in &components {
            let amount = effective_cost_amount(c, x_val);
            match c.source {
                CostSource::Hand => hand_need += amount,
                CostSource::Mill => mill_need += amount,
                CostSource::Graveyard => gy_need += amount,
                CostSource::Sacrifice | CostSource::SelfExile | CostSource::Attached => {
                    return Err(ActivateError::CannotPayComponents);
                }
            }
        }
        let p = self.player(controller);
        if p.hand.len() < hand_need
            || p.deck.len() < mill_need
            || p.graveyard.len() < gy_need
        {
            return Err(ActivateError::CannotPayComponents);
        }

        // Expose the X value to both validate and effect handlers via
        // `game.x_value()`. Saved/restored around the entire
        // validate→pay→effect sequence so a card's validate hook can
        // refuse based on X-dependent math (e.g., dark-salamander's
        // "2Y - X must be > 0").
        let prior_x = self.current_activation_x;
        self.current_activation_x = x_value;

        // RULES A.9: optional `validate` hook. If present, the activation
        // can only be initiated when validate returns truthy — typically
        // "a legal target exists." No cost is paid if validate refuses.
        // Without ctx (engine calls without a Lua VM), validate is
        // skipped — caller's responsibility, used by some tests.
        if let Some(v_fn) = validate {
            if let Some(c) = ctx.as_deref_mut() {
                if !lua_api::fire_validate(c.lua, self, c.oracle(), iid, v_fn) {
                    self.current_activation_x = prior_x;
                    return Err(ActivateError::NoLegalTarget);
                }
            }
        }

        // Pay tap cost.
        if cost_tap {
            self.set_tapped(iid, true);
        }

        // Pay component costs. HAND uses the same smart-discard ranking
        // as `game.discard` (least-useful first). MILL takes top of own
        // deck. GRAVEYARD moves cards from GY to EXILE (matching the
        // play-card convention — graveyard payments don't recycle).
        for c in &components {
            let amount = effective_cost_amount(c, x_val);
            match c.source {
                CostSource::Hand => {
                    lua_api::do_smart_discard(self, controller, amount);
                }
                CostSource::Mill => {
                    for _ in 0..amount {
                        if let Some(top) = self.player(controller).deck.first().cloned() {
                            let _ = self.move_card(
                                &top,
                                controller,
                                super::state::Zone::Deck,
                                super::state::Zone::Graveyard,
                            );
                            self.bump_action("mill", controller);
                        }
                    }
                }
                CostSource::Graveyard => {
                    for _ in 0..amount {
                        if let Some(card) = self.player(controller).graveyard.first().cloned() {
                            let _ = self.move_card(
                                &card,
                                controller,
                                super::state::Zone::Graveyard,
                                super::state::Zone::Exile,
                            );
                        }
                    }
                }
                _ => unreachable!("sacrifice / self-exile rejected at validation"),
            }
        }

        // Telemetry: bump per-controller action count so HTML reports
        // can show "X activations per game" alongside plays, attacks,
        // and engine actions. Keyed plainly as "activate" so it sums
        // across all activated abilities.
        self.bump_action("activate", controller);

        // Fire effect. Per A.5 this is inline / synchronous; the
        // handler returning is the end of the activation. The X value
        // remains visible via `game.x_value()` (set above before the
        // validate hook).
        if let Some(c) = ctx {
            lua_api::fire_activated(c.lua, self, c.oracle(), iid, handler);
        }

        self.current_activation_x = prior_x;
        Ok(())
    }

    /// Read-only eligibility check for the sim AI's activation pass.
    /// Returns true iff a subsequent `activate_ability(iid, ability_idx)`
    /// call would succeed. Matches `activate_ability`'s validation
    /// exactly so the AI never speculatively calls and fails.
    pub fn can_activate(&self, iid: &InstanceId, ability_idx: usize) -> bool {
        // Permissive pre-check: treats is_x components as "affordable
        // at X=1." The exact X is chosen by the caller (sim AI) and
        // re-validated inside `activate_ability`. Returns true here
        // when the AI should consider this activation; the AI is
        // expected to follow up with a concrete x_value if needed.
        self.can_activate_with_x(iid, ability_idx, 1)
    }

    /// Like `can_activate` but checks affordability for a specific
    /// X value. Useful for the sim AI when it wants to commit to a
    /// specific X before calling `activate_ability`.
    pub fn can_activate_with_x(
        &self,
        iid: &InstanceId,
        ability_idx: usize,
        x_value: i32,
    ) -> bool {
        let Some(inst) = self.card_pool.get(iid) else {
            return false;
        };
        let Some(ability) = self.activation_at(iid, ability_idx) else {
            return false;
        };
        if !self.player(inst.controller).board.contains(iid) {
            return false;
        }
        // RULES P.32: declarative target category — refuse if no legal
        // target exists. Mirrors the engine's activate_ability gate.
        if let Some(target) = ability.target {
            if !self.is_target_legal(target) {
                return false;
            }
        }
        if ability.cost_tap {
            if inst.tapped {
                return false;
            }
            let is_creature = inst.card.kind == CardType::Creature;
            if is_creature && inst.summoning_sick && !self.has_keyword(iid, "haste") {
                return false;
            }
        }
        // Component-cost affordability with the supplied X value.
        // is_x components multiply by x_value.
        let x = x_value.max(0);
        let mut hand_need = 0usize;
        let mut mill_need = 0usize;
        let mut gy_need = 0usize;
        for c in &ability.cost_components {
            let amount = effective_cost_amount(c, x);
            match c.source {
                CostSource::Hand => hand_need += amount,
                CostSource::Mill => mill_need += amount,
                CostSource::Graveyard => gy_need += amount,
                CostSource::Sacrifice | CostSource::SelfExile | CostSource::Attached => return false,
            }
        }
        let p = self.player(inst.controller);
        p.hand.len() >= hand_need
            && p.deck.len() >= mill_need
            && p.graveyard.len() >= gy_need
    }
}

/// Per-component effective amount: `is_x` components multiply by the
/// activation's X value; non-X components use the printed `amount`.
fn effective_cost_amount(c: &crate::card::CostComponent, x_value: i32) -> usize {
    if c.is_x {
        x_value.max(0) as usize
    } else {
        c.amount.max(0) as usize
    }
}

#[cfg(test)]
#[path = "play_tests.rs"]
mod tests;
