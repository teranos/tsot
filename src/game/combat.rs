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
    BlockerNotOnBoard,
    BlockerNotControlled,
    BlockerTapped,
    BlockerIsAttacker,
    AttackerNotDeclared,
    AttackerUnblockable,
    FlyingMustBeBlockedByFlyer,
    BlockerAlreadyAssigned,
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
        if inst.has_keyword("defender") {
            return Err(CombatError::AttackerIsDefender);
        }
        if inst.summoning_sick && !inst.has_keyword("haste") {
            return Err(CombatError::AttackerSummoningSick);
        }

        // Snapshot before mutating.
        let vigilant = inst.has_keyword("vigilance");

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

        if let Some(c) = ctx {
            lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnAttack, attacker);
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
        if blocker == attacker {
            return Err(CombatError::BlockerIsAttacker);
        }
        let blocker_has_flying = blocker_inst.has_keyword("flying");

        let attacker_inst = self
            .card_pool
            .get(attacker)
            .ok_or(CombatError::AttackerNotDeclared)?;
        if attacker_inst.has_keyword("unblockable") {
            return Err(CombatError::AttackerUnblockable);
        }
        let attacker_has_flying = attacker_inst.has_keyword("flying");

        // B.11: if attacker has flying, blocker must have flying.
        // (The "explicit anti-flying" exception has no anchor in the current corpus.)
        if attacker_has_flying && !blocker_has_flying {
            return Err(CombatError::FlyingMustBeBlockedByFlyer);
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
            );
        }
        if let Some(c) = ctx.as_mut() {
            lua_api::fire_with_partner(
                c.lua,
                self,
                c.oracle(),
                EventName::OnBlock,
                blocker,
                attacker,
            );
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
        Ok(self.resolve_combat(attacks, ctx))
    }

    fn resolve_combat(
        &mut self,
        attacks: Vec<AttackDecl>,
        ctx: Option<&mut EventContext>,
    ) -> CombatOutcome {
        let mut outcome = CombatOutcome::default();
        let defender = self.active_player.opponent();

        for atk in &attacks {
            let attacker_x = self.effective_stats(&atk.attacker).0;
            if atk.blockers.is_empty() {
                // B.6 unblocked → B.2 mill defender's DECK to EXILE.
                let mill_n = (attacker_x.max(0) as usize).min(self.player(defender).deck.len());
                outcome.defender_milled_to_exile += mill_n as i32;
                for _ in 0..mill_n {
                    let Some(top) = self.player(defender).deck.first().cloned() else {
                        break;
                    };
                    let _ = self.move_card(&top, defender, Zone::Deck, Zone::Exile);
                }
                if self.player(defender).deck.is_empty() {
                    self.set_winner(Some(defender.opponent()));
                }
            } else {
                // B.7: attacker deals X to each blocker; each blocker deals their X to attacker.
                let mut attacker_dmg = 0i32;
                for bid in &atk.blockers {
                    let blocker_x = self.effective_stats(bid).0;
                    attacker_dmg += blocker_x;
                    let current = self.card_pool.get(bid).map(|i| i.damage).unwrap_or(0);
                    self.set_damage(bid, current + attacker_x);
                }
                let current = self
                    .card_pool
                    .get(&atk.attacker)
                    .map(|i| i.damage)
                    .unwrap_or(0);
                self.set_damage(&atk.attacker, current + attacker_dmg);
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
            let damage = self.card_pool.get(iid).map(|i| i.damage).unwrap_or(0);
            let y = self.effective_stats(iid).1;
            if damage > 0 && damage >= y {
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
            outcome.deaths.push(iid.clone());
            // LUA Phase 1: fire on_die after the Board → Graveyard move so the
            // handler observes the post-death zone state. Handlers may return
            // attached cards via game.move; P.8 (auto-exile of leftover
            // attached) is still TODO and will run after handlers when wired.
            if let Some(c) = ctx.as_mut() {
                lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnDie, iid);
            }
            // TODO(types): P.8 — when a card with attached cards moves to GRAVEYARD,
            // any attached cards still present must move to EXILE.
        }
        outcome
    }
}

#[cfg(test)]
#[path = "combat_tests.rs"]
mod tests;
