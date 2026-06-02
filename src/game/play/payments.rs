//! Payment-selection helpers used by the cast / activate paths and
//! the sim AI. Pure reads of `GameState` — they don't mutate state,
//! they just pick which iids to use as payments. Extracted from
//! `play.rs` so the cast loop reads in one screen.
//!
//! - `card_identity` (P.7a identity = colors ∪ symbols)
//! - Jewel-tap helpers (P.24)
//! - `identity_matching_hand_count` (sim AI affordability)
//! - `find_attached_payments` (P.31)
//! - `resolve_graveyard_payment` (P.12 + P.12a color-anchor)
//! - `find_gy_hand_substitutes` (Clear View pattern)
//! - `resolve_hand_payment` (P.7a-filtered hand pick)

use std::collections::BTreeSet;

use super::super::state::{GameState, InstanceId, PlayerId};
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
            for sym in &inst.card.symbols {
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
                    .map(|i| i.card.gy_hand_substitute)
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
}
