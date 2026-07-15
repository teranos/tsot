//! Payment-selection helpers used by the cast / activate paths and
//! the sim AI. Pure reads of `GameState` — they don't mutate state,
//! they just pick which iids to use as payments. Extracted from
//! `play.rs` so the cast loop reads in one screen.
//!
//! - `card_identity` (P.7a identity = colors ∪ symbols)
//! - Jewel-tap helpers (P.24)
//! - `hand_/attached_/mutation_..._item_error` — the per-item payment
//!   eligibility predicates shared by `play_card` and the `eligible_*`
//!   set-builders (single source of truth; picker can't offer what the
//!   validator refuses)
//! - `find_attached_payments` (P.31)
//! - `resolve_graveyard_payment` (P.12 + P.12a color-anchor)
//! - `find_gy_hand_substitutes` (Clear View pattern)
//! - `resolve_hand_payment` (P.7a-filtered hand pick)

use std::collections::BTreeSet;

use super::super::state::{GameState, InstanceId, PlayerId};
use super::PlayError;
use crate::choice::{ChoiceOracle, ChooseCardRequest};

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
            for sym in &inst.card().symbols {
                if !sym.is_empty() {
                    ident.insert(sym.clone());
                }
            }
        }
        ident
    }

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
            .card()
            .subtypes
            .iter()
            .any(|s| s.eq_ignore_ascii_case("jewel"));
        let is_crystal = tap_card
            .card()
            .subtypes
            .iter()
            .any(|s| s.eq_ignore_ascii_case("crystal"));
        if is_jewel {
            return self
                .effective_colors(tap_iid)
                .iter()
                .any(|c| cast_colors.contains(c));
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

    /// First untapped Symbol on `player`'s BOARD that's a valid P.24e
    /// tap substitute for ANY cast (no color requirement). Used by
    /// both the picker (`sim/ai.rs::can_pay_instant_cost`) and the
    /// builder (`sim/run.rs::build_pattern_b_choices`) so the two
    /// agree on coverage and play_card never sees a Symbol-tap the
    /// picker offered but the builder didn't fill in.
    pub fn find_symbol_tap_candidate(&self, player: PlayerId) -> Option<InstanceId> {
        self.player(player)
            .board
            .iter()
            .find(|iid| {
                self.card_pool
                    .get(*iid)
                    .map(|i| {
                        matches!(i.card().kind, crate::card::CardType::Symbol)
                            && !i.tapped
                            && i.controller == player
                    })
                    .unwrap_or(false)
            })
            .cloned()
    }

    // ---- Per-item payment eligibility: the single source of truth ----
    //
    // These three predicates are the ONLY definition of "may this one id
    // pay/attach for this cast." `play_card`'s validation loop calls them,
    // and the `eligible_*` set-builders below filter on them. Because the
    // picker and resolver build payments exclusively from those builders,
    // this makes per-item drift between picker and validator structurally
    // impossible — there is nothing to keep "in agreement" by hand.
    //
    // Set-level rules (coverage/count, duplicate detection, the all-
    // cardless-can't-anchor gate) are NOT per-item facts and stay in
    // `play_card`.

    /// Per-item HAND-payment eligibility (P.6 / P.7a). `Some(err)` is the
    /// exact error `play_card` would return for this id; `None` = eligible.
    ///
    /// - `gy_anchor`: a color-matching GRAVEYARD pitch was supplied for
    ///   this cast, so P.12b suspends the per-card P.7a identity match.
    /// - `allow_cardless_body`: treat a cardless sleeve (Z.8c) as a
    ///   generic wildcard body, exempt from P.7a. `play_card` passes
    ///   `true`; the sim helper passes `false` so the picker stays
    ///   conservative (it does not yet assemble cardless bodies for an
    ///   *identity* HAND cost — the slice-8.2 deferral). Both flags are
    ///   monotonic: `false` is strictly more restrictive than `true`, so
    ///   the helper's offer set is always a subset of what `play_card`
    ///   accepts. C.14's frame gate is gone entirely — no transparency
    ///   check here.
    pub fn hand_payment_item_error(
        &self,
        cast_iid: &InstanceId,
        hid: &InstanceId,
        player: PlayerId,
        gy_anchor: bool,
        allow_cardless_body: bool,
    ) -> Option<PlayError> {
        if hid == cast_iid {
            return Some(PlayError::HandPaymentInvalid(hid.clone()));
        }
        if !self.player(player).hand.contains(hid) {
            return Some(PlayError::HandPaymentInvalid(hid.clone()));
        }
        // P.24: a static restriction can forbid a card as a HAND cost.
        if self.has_restriction(hid, crate::card::Restriction::CannotBeCostPaid) {
            return Some(PlayError::HandPaymentForbidden(hid.clone()));
        }
        // P.7a identity, unless a GY anchor suspends it (P.12b) or the
        // body is an exempt cardless sleeve (Z.8c).
        let cast_ident = self.card_identity(cast_iid);
        let needs_identity_match = !(cast_ident.is_empty()
            || gy_anchor
            || (allow_cardless_body && self.is_cardless(hid)));
        if needs_identity_match {
            let pay_ident = self.card_identity(hid);
            if cast_ident.is_disjoint(&pay_ident) {
                return Some(PlayError::HandPaymentIdentityMismatch(hid.clone()));
            }
        }
        None
    }

    /// Per-item ATTACHED-payment eligibility (P.31). `Some(err)` mirrors
    /// `play_card`; `None` = eligible. C.14's frame gate is lifted, so
    /// the only rule left is host-on-your-BOARD-and-controlled-by-you.
    pub fn attached_payment_item_error(
        &self,
        aid: &InstanceId,
        player: PlayerId,
    ) -> Option<PlayError> {
        let ok = match self.host_of(aid) {
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
        if ok {
            None
        } else {
            Some(PlayError::AttachedPaymentInvalid(aid.clone()))
        }
    }

    /// Per-item Mutation-target eligibility (P.26 / Z.7). `Some(err)`
    /// mirrors `play_card`; `None` = eligible. C.14's frame gate is
    /// lifted — a transparent mutation attaches to any creature.
    pub fn mutation_target_item_error(&self, target: &InstanceId) -> Option<PlayError> {
        let on_board = self.a.board.contains(target) || self.b.board.contains(target);
        let is_creature = self
            .card_pool
            .get(target)
            .map(|i| i.card().kind == crate::card::CardType::Creature)
            .unwrap_or(false);
        if !on_board || !is_creature {
            return Some(PlayError::MutationTargetInvalid(target.clone()));
        }
        if self.has_restriction(target, crate::card::Restriction::CannotBeAttachedTo) {
            return Some(PlayError::MutationTargetInvalid(target.clone()));
        }
        // Z.7: a sleeve holds at most 4 cards (host + 3 fused). A 4th
        // mutation is refused.
        let fused = self
            .card_pool
            .get(target)
            .map(|i| i.same_sleeve.len())
            .unwrap_or(0);
        if fused >= 3 {
            return Some(PlayError::SleeveFull(target.clone()));
        }
        None
    }

    /// The canonical mutation-target set the picker
    /// (sim/ai.rs::enumerate_playable_in_hand) and resolver
    /// (sim/run.rs::build_pattern_b_choices) both draw from. Every BOARD
    /// creature is run through `mutation_target_item_error` — the same
    /// per-item predicate play_card validates with — so play_card never
    /// sees a target the picker offered. Eligible = on-board creature,
    /// not `CannotBeAttachedTo`, sleeve not full (Z.7). C.14's frame gate
    /// is lifted, so a transparent mutation attaches to any creature.
    pub fn eligible_mutation_targets(&self, cast_iid: &InstanceId) -> Vec<InstanceId> {
        let _ = cast_iid;
        // Filter every BOARD creature through the same per-item predicate
        // play_card validates with — so the picker can never offer a
        // target play_card would refuse.
        self.a
            .board
            .iter()
            .chain(self.b.board.iter())
            .filter(|t| self.mutation_target_item_error(t).is_none())
            .cloned()
            .collect()
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

    /// SHARED predicate — the canonical ATTACHED-payment eligibility
    /// set for a cast. Both the sim AI's affordability check and the
    /// resolver's pool must use this so play_card's ATTACHED-payment
    /// validation (game/play.rs:619-650) never sees an attached iid
    /// the picker offered.
    ///
    /// Every attached id is run through `attached_payment_item_error` —
    /// the same per-item predicate play_card validates with — so the
    /// picker can never offer an attached payment play_card would refuse.
    /// (C.14 lifted: frame no longer excludes transparent attached cards.)
    pub fn eligible_attached_payments(
        &self,
        player: PlayerId,
        cast_iid: &InstanceId,
    ) -> Vec<InstanceId> {
        let _ = cast_iid;
        let mut out = Vec::new();
        for host_iid in &self.player(player).board {
            let Some(host) = self.card_pool.get(host_iid) else { continue };
            for aid in &host.attached {
                if self.attached_payment_item_error(aid, player).is_none() {
                    out.push(aid.clone());
                }
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
                i.card()
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
                        i.card()
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

    /// Pick up to `max_count` Clear View-style GY-substitute cards
    /// from `player`'s graveyard, in graveyard order. Each returned
    /// iid is a card with `Card.gy_hand_substitute = true` and lives
    /// in the controller's GRAVEYARD. The sim AI uses these to fill
    /// HAND slots that the hand's identity-matching cards can't cover.
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
                    .map(|i| i.card().gy_hand_substitute)
                    .unwrap_or(false)
            })
            .take(max_count)
            .cloned()
            .collect()
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
    /// The canonical hand-payment eligibility set the picker and
    /// resolver both draw from. Every candidate is run through
    /// `hand_payment_item_error` — the same per-item predicate
    /// `play_card` validates with — so an offered id can never be one
    /// `play_card` refuses.
    ///
    /// Called with `gy_anchor = false, allow_cardless_body = false`: the
    /// most restrictive settings, so the offer set is a strict subset of
    /// what `play_card` accepts (which may relax both). That subset is the
    /// safety property — the sim stays conservative (no cardless bodies
    /// for an identity cast, no anticipating a GY anchor) and never loops.
    pub fn eligible_hand_payments(
        &self,
        player: PlayerId,
        cast_iid: &InstanceId,
    ) -> Vec<InstanceId> {
        self.player(player)
            .hand
            .iter()
            .filter(|hid| {
                self.hand_payment_item_error(cast_iid, hid, player, false, false)
                    .is_none()
            })
            .cloned()
            .collect()
    }

    pub fn resolve_hand_payment(
        &self,
        player: PlayerId,
        instance: &InstanceId,
        hand_needed: usize,
        oracle: &mut dyn ChoiceOracle,
    ) -> Result<Vec<InstanceId>, crate::choice::ChoicePending> {
        // Use the shared predicate. Per-slot we then exclude
        // already-picked iids so a single card doesn't fill two
        // slots.
        let eligible = self.eligible_hand_payments(player, instance);
        let mut chosen: Vec<InstanceId> = Vec::with_capacity(hand_needed);
        let mut picked_set: BTreeSet<InstanceId> = BTreeSet::new();
        for slot in 0..hand_needed {
            let pool: Vec<InstanceId> = eligible
                .iter()
                .filter(|iid| !picked_set.contains(*iid))
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
                .choose_card(self, req)?
                .unwrap_or_else(|| pool_for_fallback[0].clone());
            picked_set.insert(pick.clone());
            chosen.push(pick);
        }
        Ok(chosen)
    }
}
