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
}

impl GameState {
    /// Card identity for HAND-cost matching: the set of lowercase
    /// colors plus the `symbol` (if non-empty). A card with no colors
    /// and no symbol returns an empty set — it acts as a wildcard on
    /// either side of a HAND-cost match.
    pub fn card_identity(&self, iid: &InstanceId) -> BTreeSet<String> {
        let mut ident = BTreeSet::new();
        if let Some(inst) = self.card_pool.get(iid) {
            for color in &inst.card.colors {
                ident.insert(color.to_ascii_lowercase());
            }
            if !inst.card.symbol.is_empty() {
                ident.insert(inst.card.symbol.clone());
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
            CardType::Creature | CardType::Spell | CardType::Artifact | CardType::Mutation
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
            // Identity match: discard must share ≥1 color OR the
            // (non-empty) symbol with the casting card. Colorless +
            // no-symbol CASTS are wildcards (take any discard);
            // colorless + no-symbol discards are NOT — they must
            // still find identity overlap, which empty sets can't,
            // so they can't pay for any identified card.
            let cast_ident = self.card_identity(instance);
            if !cast_ident.is_empty() {
                let pay_ident = self.card_identity(hid);
                if cast_ident.is_disjoint(&pay_ident) {
                    return Err(PlayError::HandPaymentIdentityMismatch(hid.clone()));
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
                let target = choices
                    .mutation_target
                    .as_ref()
                    .expect("validated by play_card");
                let _ = self.remove_from_zone(instance, player, Zone::Hand);
                self.add_attached(target, instance);
                self.set_face_down(instance, true);

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
        mut ctx: Option<&mut EventContext>,
    ) -> Result<(), ActivateError> {
        // First pass: read everything we need from the card_pool entry,
        // then release the borrow. All subsequent steps may mutate
        // self (set_tapped, smart-discard, fire_validate, etc.) — they
        // can't coexist with the immutable borrow on `inst`/`ability`.
        let (
            controller,
            is_creature,
            inst_tapped,
            inst_summoning_sick,
            cost_tap,
            components,
            handler,
            validate,
        ) = {
            let inst = self
                .card_pool
                .get(iid)
                .ok_or(ActivateError::SourceMissing)?;
            let ability = inst
                .card
                .activated
                .get(ability_idx)
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
            )
        };

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

        // Component-cost gate: pre-validate every component is payable
        // from the controller's current zones. Once we pass this, the
        // payment loop below cannot fail half-way through.
        let mut hand_need = 0usize;
        let mut mill_need = 0usize;
        let mut gy_need = 0usize;
        for c in &components {
            let amount = c.amount.max(0) as usize;
            match c.source {
                CostSource::Hand => hand_need += amount,
                CostSource::Mill => mill_need += amount,
                CostSource::Graveyard => gy_need += amount,
                CostSource::Sacrifice | CostSource::SelfExile => {
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

        // RULES A.9: optional `validate` hook. If present, the activation
        // can only be initiated when validate returns truthy — typically
        // "a legal target exists." No cost is paid if validate refuses.
        // Without ctx (engine calls without a Lua VM), validate is
        // skipped — caller's responsibility, used by some tests.
        if let Some(v_fn) = validate {
            if let Some(c) = ctx.as_deref_mut() {
                if !lua_api::fire_validate(c.lua, self, c.oracle(), iid, v_fn) {
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
            let amount = c.amount.max(0) as usize;
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
        // handler returning is the end of the activation.
        if let Some(c) = ctx {
            lua_api::fire_activated(c.lua, self, c.oracle(), iid, handler);
        }
        Ok(())
    }

    /// Read-only eligibility check for the sim AI's activation pass.
    /// Returns true iff a subsequent `activate_ability(iid, ability_idx)`
    /// call would succeed. Matches `activate_ability`'s validation
    /// exactly so the AI never speculatively calls and fails.
    pub fn can_activate(&self, iid: &InstanceId, ability_idx: usize) -> bool {
        let Some(inst) = self.card_pool.get(iid) else {
            return false;
        };
        let Some(ability) = inst.card.activated.get(ability_idx) else {
            return false;
        };
        if !self.player(inst.controller).board.contains(iid) {
            return false;
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
        // Component-cost affordability. Matches `activate_ability`'s
        // pre-payment validation exactly.
        let mut hand_need = 0usize;
        let mut mill_need = 0usize;
        let mut gy_need = 0usize;
        for c in &ability.cost_components {
            let amount = c.amount.max(0) as usize;
            match c.source {
                CostSource::Hand => hand_need += amount,
                CostSource::Mill => mill_need += amount,
                CostSource::Graveyard => gy_need += amount,
                CostSource::Sacrifice | CostSource::SelfExile => return false,
            }
        }
        let p = self.player(inst.controller);
        p.hand.len() >= hand_need
            && p.deck.len() >= mill_need
            && p.graveyard.len() >= gy_need
    }
}

#[cfg(test)]
#[path = "play_tests.rs"]
mod tests;
