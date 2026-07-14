//! Combat: attack declaration, block declaration, damage resolution.
//!
//! Mirrors RULES.md sections B (combat) and references P.4 / P.8 for death movement.
//!
//! Scope of this slice:
//!   - declare_attacker / confirm_attacks
//!   - declare_blocker / confirm_blocks (calls resolve internally)
//!   - resolve: B.2 (unblocked → mill defender to EXILE), B.7 (damage exchange), B.8 (deaths)
//!
//! Deferred:
//!   - Response windows (R.1) inside combat — no instants implemented yet.
//!   - Triggered abilities ("when this attacks", "when this dies", etc.).
//!   - P.8 attached-cards-go-to-exile on host death.
//!   - Control changes (T.1) — caller treats owner == controller.

use super::context::EventContext;
use super::lua_api;
use super::state::{AttackDecl, CombatState, GameState, InstanceId, Phase, Zone};
use crate::card::EventName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatError {
    /// A Lua handler fired during combat (on_attack / on_block / on_blocked_by
    /// / on_die from combat damage / on_creature_dies broadcast / on-attached
    /// for attackers that landed damage) called `game.choose_card` /
    /// `game.confirm` / `game.choose_player` / `game.choose_int` with an
    /// oracle that needs the human to answer. Mirrors
    /// `PlayError::ChoicePending` — wrapper raises `Error::external(_)`,
    /// `fire_*` downcasts, the combat method lifts via `?`. The StepEngine
    /// catches this variant, rolls back the preview journal, surfaces a
    /// `HumanPrompt::Choose*`, and re-fires after the user's answer is
    /// appended to `HumanReplayOracle.replay`.
    ChoicePending(crate::choice::ChoicePending),
    GameOver,
    NotCombatPhase,
    NotAwaitingAttackers,
    NotAwaitingBlockers,
    AttackerNotOnBoard,
    AttackerNotControlled,
    AttackerTapped,
    AttackerSummoningSick,
    AttackerIsDefender,
    AttackerAlreadyDeclared,
    /// Phase 3: a restriction static (e.g., flesh-eating-plant's
    /// insects-cannot-attack) blocks this attacker from being declared.
    AttackerForbiddenByRestriction,
    BlockerNotOnBoard,
    BlockerNotControlled,
    BlockerTapped,
    BlockerCannotBlock,
    BlockerIsAttacker,
    AttackerNotDeclared,
    AttackerUnblockable,
    FlyingMustBeBlockedByFlyer,
    BlockerAlreadyAssigned,
    /// Blocker's `cannot_block_subtypes` includes one of the
    /// attacker's subtypes (case-insensitive). Used by rats vs cats.
    BlockerCannotBlockSubtype,
}

/// Summary of a combat resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CombatOutcome {
    /// Total cards exiled from the defending player's DECK by unblocked attacks (per B.2).
    pub defender_milled_to_exile: i32,
    /// Creatures that died this combat (sent to GRAVEYARD per P.4).
    pub deaths: Vec<InstanceId>,
}

impl GameState {
    /// Declare one attacker. Taps the creature unless it has vigilance.
    /// Validates B.3 (summoning sick unless **haste**), B.13 (not tapped),
    /// B.17 (not **defender**), and that the creature is on the active player's BOARD.
    ///
    /// When `lua` is `Some`, the attacker's `on_attack` handler fires after the
    /// attack is recorded.
    pub fn declare_attacker(
        &mut self,
        attacker: &InstanceId,
        ctx: Option<&mut EventContext>,
    ) -> Result<(), CombatError> {
        if self.winner.is_some() {
            return Err(CombatError::GameOver);
        }
        if self.phase != Phase::Combat {
            return Err(CombatError::NotCombatPhase);
        }
        // First declare initializes combat state; subsequent declares append to
        // the buffered attacker list. AwaitingBlockers (the data-shape used as
        // a buffer during declaration) is also a valid pre-confirm state.
        match &self.combat {
            None => self.set_combat(Some(CombatState::AwaitingAttackers)),
            Some(CombatState::AwaitingAttackers)
            | Some(CombatState::AwaitingBlockers { .. }) => {}
        }

        let active = self.active_player;
        if !self.player(active).board.contains(attacker) {
            return Err(CombatError::AttackerNotOnBoard);
        }

        let inst = self
            .card_pool
            .get(attacker)
            .ok_or(CombatError::AttackerNotOnBoard)?;
        if inst.controller != active {
            return Err(CombatError::AttackerNotControlled);
        }
        if inst.tapped {
            return Err(CombatError::AttackerTapped);
        }
        if self.has_keyword(attacker, "defender") {
            return Err(CombatError::AttackerIsDefender);
        }
        if inst.summoning_sick && !self.has_keyword(attacker, "haste") {
            return Err(CombatError::AttackerSummoningSick);
        }
        if self.has_restriction(attacker, crate::card::Restriction::CannotAttack) {
            return Err(CombatError::AttackerForbiddenByRestriction);
        }

        // Snapshot before mutating.
        let vigilant = self.has_keyword(attacker, "vigilance");

        // Append to the buffered attacker list (clone-modify-set so the
        // mutation goes through the journal). The buffer is encoded as
        // AwaitingBlockers from the start (data shape matches);
        // confirm_attacks is the no-op marker that says "no more attackers,
        // blockers may now declare."
        let new_combat = match self.combat.clone() {
            Some(CombatState::AwaitingAttackers) => CombatState::AwaitingBlockers {
                attacks: vec![AttackDecl {
                    attacker: attacker.clone(),
                    blockers: Vec::new(),
                }],
            },
            Some(CombatState::AwaitingBlockers { mut attacks }) => {
                if attacks.iter().any(|a| &a.attacker == attacker) {
                    return Err(CombatError::AttackerAlreadyDeclared);
                }
                attacks.push(AttackDecl {
                    attacker: attacker.clone(),
                    blockers: Vec::new(),
                });
                CombatState::AwaitingBlockers { attacks }
            }
            None => unreachable!(),
        };
        self.set_combat(Some(new_combat));

        // Tap unless vigilance.
        if !vigilant {
            self.set_tapped(attacker, true); // B.4
        }

        // Record that combat happened this turn (read by cards whose effect
        // scales with whether anyone attacked). Cleared on End → Untap.
        self.set_creature_attacked_this_turn(true);
        // Per-instance attack tracking for activated abilities that
        // condition on "if THIS creature attacked this turn" (vigilant-
        // human's T-ability is the first user). Also cleared at turn start.
        self.set_attacked_this_turn(attacker, true);

        // R.1.b — open a response window with empty chain. Defender gets
        // their chance (after active passes) to cast removal / counters /
        // pumps before on_attack fires. The window has no chain item: the
        // attack declaration itself is already a fait accompli (creature
        // tapped, recorded in combat state). Window closes naturally after
        // two consecutive passes on an empty chain. on_attack then fires
        // inline as part of the resolution sequence.
        let mut ctx = ctx;
        let _ = self.open_response_window_empty();
        let _ = self.drive_window_to_close(ctx.as_deref_mut());

        // OnAttack fires on the attacker, then on every card attached to
        // the attacker. Mirror of the OnDealtDamageToPlayer iteration
        // below — mutations like TNF / VEGF declare on_attack handlers and
        // expect to receive `self = the mutation` when the host attacks.
        // Snapshot attached before firing so a handler that detaches /
        // moves doesn't desync the iteration.
        // Z.7: fused same-sleeve mutations (klotho / TNF / VEGF) declare
        // OnAttack and receive `self = the mutation`, so iterate the whole
        // unit (attached ∪ same_sleeve), not just attached.
        let attached: Vec<InstanceId> = self
            .card_pool
            .get(attacker)
            .map(|i| i.children().cloned().collect())
            .unwrap_or_default();
        if let Some(c) = ctx.as_deref_mut() {
            lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnAttack, attacker)
                .map_err(CombatError::ChoicePending)?;
        }
        for aid in &attached {
            if let Some(c) = ctx.as_deref_mut() {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnAttack, aid)
                    .map_err(CombatError::ChoicePending)?;
            }
        }

        Ok(())
    }

    /// Finalize attackers. After this, no more attackers may be added; blockers may now be declared.
    /// (Mostly a marker — in this slice it just validates we're in a state where attackers exist.)
    pub fn confirm_attacks(&mut self) -> Result<(), CombatError> {
        if self.phase != Phase::Combat {
            return Err(CombatError::NotCombatPhase);
        }
        match &self.combat {
            Some(CombatState::AwaitingBlockers { .. }) => Ok(()),
            _ => Err(CombatError::NotAwaitingAttackers),
        }
    }

    /// Declare one blocker against one specific attacker.
    /// Validates B.12 (not tapped), B.14 (attacker not **unblockable**),
    /// B.11 (flying restriction), and that the blocker is on the defending player's BOARD.
    ///
    /// `lua` is the long-lived VM from `CardRegistry`. When `Some`, the
    /// attacker's `on_blocked_by` handler fires per blocker. When `None`,
    /// handlers are skipped (used by unit tests of pure combat logic).
    pub fn declare_blocker(
        &mut self,
        blocker: &InstanceId,
        attacker: &InstanceId,
        ctx: Option<&mut EventContext>,
    ) -> Result<(), CombatError> {
        if self.winner.is_some() {
            return Err(CombatError::GameOver);
        }
        if self.phase != Phase::Combat {
            return Err(CombatError::NotCombatPhase);
        }
        let defender = self.active_player.opponent();

        if !self.player(defender).board.contains(blocker) {
            return Err(CombatError::BlockerNotOnBoard);
        }
        let blocker_inst = self
            .card_pool
            .get(blocker)
            .ok_or(CombatError::BlockerNotOnBoard)?;
        if blocker_inst.controller != defender {
            return Err(CombatError::BlockerNotControlled);
        }
        if blocker_inst.tapped {
            return Err(CombatError::BlockerTapped);
        }
        if self.has_keyword(blocker, "cannot-block") {
            return Err(CombatError::BlockerCannotBlock);
        }
        if blocker == attacker {
            return Err(CombatError::BlockerIsAttacker);
        }
        let blocker_has_flying = self.has_keyword(blocker, "flying");

        let _attacker_inst = self
            .card_pool
            .get(attacker)
            .ok_or(CombatError::AttackerNotDeclared)?;
        if self.has_keyword(attacker, "unblockable") {
            return Err(CombatError::AttackerUnblockable);
        }
        let attacker_has_flying = self.has_keyword(attacker, "flying");
        let blocker_has_reach = self.has_keyword(blocker, "reach");
        // Predator-prey exception: blocker.can_block_subtypes lists
        // subtypes the blocker can pin down regardless of flying
        // (e.g., cats can block birds). Empty for most cards.
        let blocker_can_subtype_override = {
            let attacker_subs: Vec<String> = self
                .card_pool
                .get(attacker)
                .map(|i| {
                    i.card
                        .subtypes
                        .iter()
                        .map(|s| s.to_ascii_lowercase())
                        .collect()
                })
                .unwrap_or_default();
            blocker_inst
                .card
                .can_block_subtypes
                .iter()
                .any(|s| attacker_subs.iter().any(|a| a == s))
        };

        // B.11: if attacker has flying, blocker must have flying OR reach
        // OR a matching subtype override (cats block birds).
        if attacker_has_flying
            && !blocker_has_flying
            && !blocker_has_reach
            && !blocker_can_subtype_override
        {
            return Err(CombatError::FlyingMustBeBlockedByFlyer);
        }

        // Subtype-block restriction: rejected if blocker's
        // `cannot_block_subtypes` intersects with attacker's subtypes.
        // Rats vs cats: rat's `cannot_block_subtypes = ["cat"]` blocks
        // (sic) the block. Case-insensitive.
        let blocker_cant: &[String] = &blocker_inst.card.cannot_block_subtypes;
        if !blocker_cant.is_empty() {
            let attacker_subs: Vec<String> = self
                .card_pool
                .get(attacker)
                .map(|i| i.card.subtypes.iter().map(|s| s.to_ascii_lowercase()).collect())
                .unwrap_or_default();
            if blocker_cant
                .iter()
                .any(|s| attacker_subs.iter().any(|a| a == s))
            {
                return Err(CombatError::BlockerCannotBlockSubtype);
            }
        }

        // Clone-modify-set so the mutation goes through the journal.
        let new_combat = match self.combat.clone() {
            Some(CombatState::AwaitingBlockers { mut attacks }) => {
                let atk = attacks
                    .iter_mut()
                    .find(|a| &a.attacker == attacker)
                    .ok_or(CombatError::AttackerNotDeclared)?;
                if atk.blockers.contains(blocker) {
                    return Err(CombatError::BlockerAlreadyAssigned);
                }
                atk.blockers.push(blocker.clone());
                CombatState::AwaitingBlockers { attacks }
            }
            _ => return Err(CombatError::NotAwaitingBlockers),
        };
        self.set_combat(Some(new_combat));

        // LUA Phase 1: fire `on_blocked_by` on the attacker, then `on_block` on the blocker.
        // Per-blocker semantics for both. Order: attacker-side first, then blocker-side.
        // Errors log and continue per LUA.md Q #3.
        let mut ctx = ctx;
        if let Some(c) = ctx.as_mut() {
            lua_api::fire_with_partner(
                c.lua,
                self,
                c.oracle(),
                EventName::OnBlockedBy,
                attacker,
                blocker,
            )
            .map_err(CombatError::ChoicePending)?;
        }
        if let Some(c) = ctx.as_mut() {
            lua_api::fire_with_partner(
                c.lua,
                self,
                c.oracle(),
                EventName::OnBlock,
                blocker,
                attacker,
            )
            .map_err(CombatError::ChoicePending)?;
        }
        // RULES R.1 lists only two window-openers: card-played and
        // attack-declared. Block declarations are atomic — no window opens
        // here. on_blocked_by / on_block fire inline as part of the
        // declaration (consistent with the "consequential triggers stay
        // inline" design). Earlier drafts proposed an R.1.c for this site;
        // dropped 2026-05-30.

        Ok(())
    }

    /// Finalize blockers and resolve combat: damage, deaths, B.2 mill.
    ///
    /// `lua` is the long-lived VM from `CardRegistry`. When `Some`, dying
    /// creatures' `on_die` handlers fire. When `None`, handlers are skipped.
    pub fn confirm_blocks(
        &mut self,
        ctx: Option<&mut EventContext>,
    ) -> Result<CombatOutcome, CombatError> {
        if self.phase != Phase::Combat {
            return Err(CombatError::NotCombatPhase);
        }
        let attacks = match self.combat.clone() {
            Some(CombatState::AwaitingBlockers { attacks }) => attacks,
            _ => return Err(CombatError::NotAwaitingBlockers),
        };
        self.set_combat(None);
        self.resolve_combat(attacks, ctx)
    }

    fn resolve_combat(
        &mut self,
        attacks: Vec<AttackDecl>,
        ctx: Option<&mut EventContext>,
    ) -> Result<CombatOutcome, CombatError> {
        let mut outcome = CombatOutcome::default();
        let defender = self.active_player.opponent();
        let mut ctx = ctx;
        // Track which attackers successfully damaged the defender's deck
        // so we can fire OnDealtDamageToPlayer on them (and their
        // attached cards — klotho-style mutations) after the mill loop.
        let mut damaged_attackers: Vec<InstanceId> = Vec::new();

        // Pass 1: blocked combats resolve per-attacker (B.7 creature-vs-
        // creature damage is fractional accumulation, B.8 compares exactly
        // — see RULES.md). Unblocked attackers are bucketed for the
        // per-combat sum-then-floor mill (B.2b).
        let mut unblocked_x_sum: f32 = 0.0;
        let mut unblocked_attackers: Vec<InstanceId> = Vec::new();
        for atk in &attacks {
            let attacker_x = self.effective_stats(&atk.attacker).0;
            if atk.blockers.is_empty() {
                // B.2b: defer the mill — sum every successful attacker's X
                // first, then floor once. A single 0.5/1 unblocked mills 0;
                // two of them mill 1.
                unblocked_x_sum += attacker_x.max(0.0);
                unblocked_attackers.push(atk.attacker.clone());
            } else {
                // B.7: attacker deals X to each blocker; each blocker deals their X to attacker.
                let mut attacker_dmg = 0.0_f32;
                for bid in &atk.blockers {
                    let blocker_x = self.effective_stats(bid).0;
                    attacker_dmg += blocker_x;
                    let current = self.card_pool.get(bid).map(|i| i.damage).unwrap_or(0.0);
                    self.set_damage(bid, current + attacker_x);
                }
                let current = self
                    .card_pool
                    .get(&atk.attacker)
                    .map(|i| i.damage)
                    .unwrap_or(0.0);
                self.set_damage(&atk.attacker, current + attacker_dmg);
            }
        }

        // Pass 2: B.2b — single combat-level mill. floor(ΣX) cards move
        // from defender's DECK to EXILE. OnDealtDamageToPlayer (P.41-ish)
        // fires for every unblocked attacker iff any card actually moved
        // — a sub-1 sum mills nothing, so no trigger.
        let mill_n = (unblocked_x_sum.floor().max(0.0) as usize)
            .min(self.player(defender).deck.len());
        outcome.defender_milled_to_exile += mill_n as i32;
        for _ in 0..mill_n {
            let Some(top) = self.player(defender).deck.first().cloned() else {
                break;
            };
            // Sacred-error sweep: combat damage mills deck-top to exile.
            // top came from deck.first() so NotInZone shouldn't happen,
            // but if it ever does it's state corruption that the typed
            // Error surfaces now instead of swallowing.
            let _ = self.move_card_or_emit(
                &top,
                defender,
                Zone::Deck,
                Zone::Exile,
                "combat-damage-mill",
            );
        }
        if mill_n > 0 {
            damaged_attackers.extend(unblocked_attackers);
        }
        if self.player(defender).deck.is_empty() {
            self.set_winner(Some(defender.opponent()), "deckout_combat_mill");
        }

        // Fire OnDealtDamageToPlayer for each attacker that successfully
        // damaged the defender. Also fires on every card attached to
        // that attacker — klotho-style mutations declare the handler
        // and receive `self` = the mutation, drawing for `self.owner`.
        for attacker in &damaged_attackers {
            // Snapshot the attached list before firing so handlers that
            // mutate state (detach, move) don't desync the iteration.
            // Z.7: same-sleeve mutations declaring OnDealtDamageToPlayer
            // (cinder-wurm et al.) fire too — iterate the whole unit.
            let attached: Vec<InstanceId> = self
                .card_pool
                .get(attacker)
                .map(|i| i.children().cloned().collect())
                .unwrap_or_default();
            if let Some(c) = ctx.as_deref_mut() {
                lua_api::fire_self_only(
                    c.lua,
                    self,
                    c.oracle(),
                    EventName::OnDealtDamageToPlayer,
                    attacker,
                )
                .map_err(CombatError::ChoicePending)?;
            }
            for aid in &attached {
                if let Some(c) = ctx.as_deref_mut() {
                    lua_api::fire_self_only(
                        c.lua,
                        self,
                        c.oracle(),
                        EventName::OnDealtDamageToPlayer,
                        aid,
                    )
                    .map_err(CombatError::ChoicePending)?;
                }
            }
        }

        // TODO(sbas): MTG-style state-based actions fire BETWEEN stack-item
        // resolutions, allowing "regenerate" / "prevent damage" responses to
        // save a dying creature. tsot currently does the damage tally and
        // death check in one pass, leaving no window. Reworking this to a
        // proper SBA loop is part of the stack work (Phase 1 or 2).
        // B.8 death check: damage ≥ effective Y → dies to GRAVEYARD per P.4.
        let on_board: Vec<InstanceId> = self
            .a
            .board
            .iter()
            .chain(self.b.board.iter())
            .cloned()
            .collect();
        let mut to_kill: Vec<InstanceId> = Vec::new();
        for iid in &on_board {
            let damage = self.card_pool.get(iid).map(|i| i.damage).unwrap_or(0.0);
            let y = self.effective_stats(iid).1;
            if damage > 0.0 && damage >= y {
                to_kill.push(iid.clone());
            }
        }
        for iid in &to_kill {
            let owner = self
                .card_pool
                .get(iid)
                .map(|i| i.owner)
                .unwrap_or(self.active_player);
            // Sacred-error sweep: board → graveyard on combat death.
            let _ = self.move_card_or_emit(
                iid,
                owner,
                Zone::Board,
                Zone::Graveyard,
                "combat-death",
            );
            outcome.deaths.push(iid.clone());
            // LUA Phase 1: fire on_die after the Board → Graveyard move so the
            // handler observes the post-death zone state. Handlers may return
            // attached cards via game.move; P.8 (auto-exile of leftover
            // attached) is still TODO and will run after handlers when wired.
            if let Some(c) = ctx.as_mut() {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnDie, iid)
                    .map_err(CombatError::ChoicePending)?;
                // Broadcast OnCreatureDies to every BOARD watcher (both
                // sides). The dying card already left BOARD above, so
                // it's naturally excluded from the snapshot.
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
                    )
                    .map_err(CombatError::ChoicePending)?;
                }
            }
            // P.8: cascade any cards still attached to the dead host
            // into EXILE. Runs AFTER on_die so handlers like
            // trustworthy-lender that want to return attached cards to
            // hand still get the first read.
            self.exile_remaining_attached(iid);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
#[path = "combat_tests.rs"]
mod tests;
