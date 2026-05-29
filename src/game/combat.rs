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

use super::lua_api;
use super::state::{AttackDecl, CombatState, GameState, InstanceId, Phase, Zone};
use mlua::Lua;

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
    pub fn declare_attacker(&mut self, attacker: &InstanceId) -> Result<(), CombatError> {
        if self.winner.is_some() {
            return Err(CombatError::GameOver);
        }
        if self.phase != Phase::Combat {
            return Err(CombatError::NotCombatPhase);
        }
        if !matches!(self.combat, Some(CombatState::AwaitingAttackers)) {
            // First declare on this turn: initialize the combat state.
            if self.combat.is_none() {
                self.combat = Some(CombatState::AwaitingAttackers);
            } else {
                return Err(CombatError::NotAwaitingAttackers);
            }
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

        // Add to current attack list (initialize the state if needed).
        if !matches!(self.combat, Some(CombatState::AwaitingAttackers)) {
            self.combat = Some(CombatState::AwaitingAttackers);
        }
        // Track attackers in a temporary holder until confirm_attacks transitions.
        // For simplicity, mutate as AwaitingBlockers-prep: append into a buffered list.
        // We re-use AwaitingBlockers-shape internally during declaration too.
        // (Cleaner: keep AwaitingAttackers state but with attacks accumulated.)
        // To avoid an extra enum variant, store accumulated attackers as a side-cache:
        // we just push into `pending_attackers` (added via this method).
        // Simpler approach: store as AwaitingBlockers immediately and just not allow blocks until confirm.
        // We'll keep AwaitingAttackers and use a small side Vec — but enum stays simple by reusing
        // AwaitingBlockers with the understanding it's not yet "confirmed".
        match &mut self.combat {
            Some(CombatState::AwaitingAttackers) => {
                // Transition into AwaitingBlockers shape with empty blockers; confirm_attacks is a no-op marker.
                // We model accumulated attackers via AwaitingBlockers because the data shape matches.
                // confirm_attacks() is required to "lock" attackers before blockers may be declared.
                // To make this work we treat the *first* declare_attacker as transitioning to
                // a "buffering" mode encoded as AwaitingBlockers with no entries.
                self.combat = Some(CombatState::AwaitingBlockers {
                    attacks: vec![AttackDecl {
                        attacker: attacker.clone(),
                        blockers: Vec::new(),
                    }],
                });
            }
            Some(CombatState::AwaitingBlockers { attacks }) => {
                if attacks.iter().any(|a| &a.attacker == attacker) {
                    return Err(CombatError::AttackerAlreadyDeclared);
                }
                attacks.push(AttackDecl {
                    attacker: attacker.clone(),
                    blockers: Vec::new(),
                });
            }
            None => unreachable!(),
        }

        // Tap unless vigilance.
        if !vigilant {
            if let Some(a) = self.card_pool.get_mut(attacker) {
                a.tapped = true; // B.4
            }
        }

        // TODO(events): fire "whenever this creature attacks" triggers per A.1.
        // E.g., squirrel-overrun's "may attach 1"; midnight-raven's "may put top of deck on bottom".
        // TODO(stack): open a response window per R.1 (attack-declared trigger).

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
        lua: Option<&Lua>,
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

        match &mut self.combat {
            Some(CombatState::AwaitingBlockers { attacks }) => {
                // Verify the attacker was declared.
                let atk = attacks
                    .iter_mut()
                    .find(|a| &a.attacker == attacker)
                    .ok_or(CombatError::AttackerNotDeclared)?;
                if atk.blockers.contains(blocker) {
                    return Err(CombatError::BlockerAlreadyAssigned);
                }
                atk.blockers.push(blocker.clone());
            }
            _ => return Err(CombatError::NotAwaitingBlockers),
        }

        // LUA Phase 1: fire `on_blocked_by` on the attacker, once per blocker.
        // Per-blocker (not once-per-attacker) — matches handler signature `(game, self, blocker)`.
        // Errors log and continue per LUA.md Q #3.
        if let Some(lua) = lua {
            lua_api::fire_on_blocked_by(lua, self, attacker, blocker);
        }
        // TODO(events): also fire `on_block` on the blocker. Skipped for now — first
        // wiring focuses on a single event end-to-end.
        // TODO(stack): when block-declaration is added as an R.1 window-opener (per design
        // discussion), open a response window here.

        Ok(())
    }

    /// Finalize blockers and resolve combat: damage, deaths, B.2 mill.
    pub fn confirm_blocks(&mut self) -> Result<CombatOutcome, CombatError> {
        if self.phase != Phase::Combat {
            return Err(CombatError::NotCombatPhase);
        }
        let attacks = match self.combat.take() {
            Some(CombatState::AwaitingBlockers { attacks }) => attacks,
            other => {
                self.combat = other;
                return Err(CombatError::NotAwaitingBlockers);
            }
        };
        Ok(self.resolve_combat(attacks))
    }

    fn resolve_combat(&mut self, attacks: Vec<AttackDecl>) -> CombatOutcome {
        let mut outcome = CombatOutcome::default();
        let defender = self.active_player.opponent();

        for atk in &attacks {
            let attacker_x = self.effective_stats(&atk.attacker).0;
            if atk.blockers.is_empty() {
                // B.6 unblocked → B.2 mill defender's DECK to EXILE.
                let mill_n = (attacker_x.max(0) as usize).min(self.player(defender).deck.len());
                outcome.defender_milled_to_exile += mill_n as i32;
                let pm = self.player_mut(defender);
                for _ in 0..mill_n {
                    let top = pm.deck.remove(0);
                    pm.exile.push(top);
                }
                if self.player(defender).deck.is_empty() {
                    self.winner = Some(defender.opponent());
                }
            } else {
                // B.7: attacker deals X to each blocker; each blocker deals their X to attacker.
                let mut attacker_dmg = 0i32;
                for bid in &atk.blockers {
                    let blocker_x = self.effective_stats(bid).0;
                    attacker_dmg += blocker_x;
                    if let Some(b) = self.card_pool.get_mut(bid) {
                        b.damage += attacker_x;
                    }
                }
                if let Some(a) = self.card_pool.get_mut(&atk.attacker) {
                    a.damage += attacker_dmg;
                }
            }
        }

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
        for iid in &to_kill {
            let owner = self
                .card_pool
                .get(iid)
                .map(|i| i.owner)
                .unwrap_or(self.active_player);
            let _ = self.move_card(iid, owner, Zone::Board, Zone::Graveyard);
            outcome.deaths.push(iid.clone());
            // TODO(events): fire "when this creature dies" triggers per A.1.
            // E.g., mesopelagic-fish, flesh-eating-plant, trustworthy-lender, attach-shuffler.
            // TODO(types): P.8 — when a card with attached cards moves to GRAVEYARD, those
            // attached cards must move to EXILE. Currently dropped on the floor.
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::state::PlayerId;
    use crate::game::test_helpers::*;

    fn put_on_board(s: &mut GameState, side: PlayerId, iid: &InstanceId) {
        s.player_mut(side).hand.retain(|x| x != iid);
        s.player_mut(side).board.push(iid.clone());
    }

    fn add_ability(s: &mut GameState, iid: &InstanceId, ability: &str) {
        s.card_pool
            .get_mut(iid)
            .unwrap()
            .card
            .abilities
            .push(ability.to_string());
    }

    fn enter_combat(s: &mut GameState) {
        // From Untap, advance 3 phases to reach Combat.
        while s.phase != Phase::Combat {
            s.next_phase();
        }
    }

    #[test]
    fn unblocked_attack_mills_defender_deck_to_exile() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        // Skip summoning sickness by giving haste, since the creature was just placed manually.
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        let defender_deck_before = s.b.deck.len();
        let defender_exile_before = s.b.exile.len();
        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();
        let outcome = s.confirm_blocks().unwrap();
        // deck_of(...) makes 1/1 cards, so attacker_x = 1.
        assert_eq!(outcome.defender_milled_to_exile, 1);
        assert_eq!(s.b.deck.len(), defender_deck_before - 1);
        assert_eq!(s.b.exile.len(), defender_exile_before + 1);
        assert!(outcome.deaths.is_empty());
    }

    #[test]
    fn blocked_attack_exchanges_damage_both_die_on_equal_stats() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();
        s.declare_blocker(&blk, &atk, None).unwrap();
        let outcome = s.confirm_blocks().unwrap();
        // Both are 1/1 — each deals 1 to other, both reach damage >= y → die.
        assert_eq!(outcome.defender_milled_to_exile, 0);
        assert!(outcome.deaths.contains(&atk));
        assert!(outcome.deaths.contains(&blk));
        assert!(s.a.graveyard.contains(&atk));
        assert!(s.b.graveyard.contains(&blk));
    }

    #[test]
    fn attacker_taps_on_declaration() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);
        s.declare_attacker(&atk).unwrap();
        assert!(s.card_pool.get(&atk).unwrap().tapped);
    }

    #[test]
    fn vigilance_attacker_does_not_tap() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        add_ability(&mut s, &atk, "vigilance");
        enter_combat(&mut s);
        s.declare_attacker(&atk).unwrap();
        assert!(!s.card_pool.get(&atk).unwrap().tapped);
    }

    #[test]
    fn defender_cannot_attack() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        add_ability(&mut s, &atk, "defender");
        enter_combat(&mut s);
        assert_eq!(
            s.declare_attacker(&atk),
            Err(CombatError::AttackerIsDefender)
        );
    }

    #[test]
    fn summoning_sick_cannot_attack() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        s.card_pool.get_mut(&atk).unwrap().summoning_sick = true;
        enter_combat(&mut s);
        assert_eq!(
            s.declare_attacker(&atk),
            Err(CombatError::AttackerSummoningSick)
        );
    }

    #[test]
    fn haste_overrides_summoning_sickness() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        s.card_pool.get_mut(&atk).unwrap().summoning_sick = true;
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);
        assert!(s.declare_attacker(&atk).is_ok());
    }

    #[test]
    fn tapped_creature_cannot_attack() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        s.card_pool.get_mut(&atk).unwrap().tapped = true;
        enter_combat(&mut s);
        assert_eq!(s.declare_attacker(&atk), Err(CombatError::AttackerTapped));
    }

    #[test]
    fn unblockable_attacker_refuses_blockers() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        add_ability(&mut s, &atk, "unblockable");
        enter_combat(&mut s);
        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();
        assert_eq!(
            s.declare_blocker(&blk, &atk, None),
            Err(CombatError::AttackerUnblockable)
        );
    }

    #[test]
    fn flying_attacker_blocked_by_flyer_succeeds() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        add_ability(&mut s, &atk, "flying");
        add_ability(&mut s, &blk, "flying");
        enter_combat(&mut s);
        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();
        assert!(s.declare_blocker(&blk, &atk, None).is_ok());
    }

    #[test]
    fn flying_attacker_refuses_ground_blocker() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        add_ability(&mut s, &atk, "flying");
        enter_combat(&mut s);
        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();
        assert_eq!(
            s.declare_blocker(&blk, &atk, None),
            Err(CombatError::FlyingMustBeBlockedByFlyer)
        );
    }

    #[test]
    fn tapped_blocker_cannot_block() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        s.card_pool.get_mut(&blk).unwrap().tapped = true;
        enter_combat(&mut s);
        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();
        assert_eq!(
            s.declare_blocker(&blk, &atk, None),
            Err(CombatError::BlockerTapped)
        );
    }

    #[test]
    fn attacker_outside_combat_phase_errors() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        // Still in Untap.
        assert_eq!(
            s.declare_attacker(&atk),
            Err(CombatError::NotCombatPhase)
        );
    }

    #[test]
    fn on_blocked_by_handler_fires_when_block_declared() {
        use crate::card::CardRegistry;
        use std::fs;

        // Write a fixture card whose on_blocked_by handler sets a Lua global,
        // so we can observe the fire from the host side.
        let tmp = std::env::temp_dir().join("tsot_on_blocked_by_test");
        fs::create_dir_all(&tmp).unwrap();
        let card_path = tmp.join("fire-on-block.lua");
        fs::write(
            &card_path,
            r#"return {
                id = "fire-on-block",
                on_blocked_by = function(game, self, blocker)
                    _G.fire_on_block_count = (_G.fire_on_block_count or 0) + 1
                end,
            }"#,
        )
        .unwrap();

        let registry = CardRegistry::load(&tmp).unwrap();
        let fixture = registry
            .cards()
            .iter()
            .find(|c| c.id == "fire-on-block")
            .unwrap()
            .clone();

        // Build a game where the fixture attacks; any vanilla creature blocks.
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        // Swap the attacker's card data for the fixture (keep stats so combat math works).
        {
            let inst = s.card_pool.get_mut(&atk).unwrap();
            inst.card.handlers = fixture.handlers.clone();
            inst.card.id = fixture.id.clone();
        }
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();
        s.declare_blocker(&blk, &atk, Some(registry.lua())).unwrap();

        let count: i32 = registry
            .lua()
            .globals()
            .get("fire_on_block_count")
            .unwrap();
        assert_eq!(count, 1);

        fs::remove_file(&card_path).ok();
    }

    #[test]
    fn tantrum_imp_handler_damages_blocker_and_mills_defender() {
        use crate::card::CardRegistry;

        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let tantrum = registry
            .cards()
            .iter()
            .find(|c| c.id == "tantrum-imp")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        // Replace attacker's card data with tantrum-imp's (handler + id),
        // keep the 1/1 stats so combat math stays predictable.
        {
            let inst = s.card_pool.get_mut(&atk).unwrap();
            inst.card.handlers = tantrum.handlers.clone();
            inst.card.id = tantrum.id.clone();
        }
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();

        let defender_deck_before = s.b.deck.len();
        let defender_exile_before = s.b.exile.len();

        s.declare_blocker(&blk, &atk, Some(registry.lua())).unwrap();

        // Handler ran during declare_blocker (before resolve_combat):
        // blocker took 1 damage; defender's deck top went to exile.
        assert_eq!(
            s.card_pool.get(&blk).unwrap().damage,
            1,
            "blocker should have 1 damage from handler"
        );
        assert_eq!(s.b.deck.len(), defender_deck_before - 1);
        assert_eq!(s.b.exile.len(), defender_exile_before + 1);
    }

    #[test]
    fn squirrel_overrun_handler_draws_a_card_when_blocked() {
        use crate::card::CardRegistry;

        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let squirrel = registry
            .cards()
            .iter()
            .find(|c| c.id == "squirrel-overrun")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&atk).unwrap();
            inst.card.handlers = squirrel.handlers.clone();
            inst.card.id = squirrel.id.clone();
        }
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();

        let a_hand_before = s.a.hand.len();
        let a_deck_before = s.a.deck.len();

        s.declare_blocker(&blk, &atk, Some(registry.lua())).unwrap();

        assert_eq!(s.a.hand.len(), a_hand_before + 1);
        assert_eq!(s.a.deck.len(), a_deck_before - 1);
        assert_eq!(s.triggered_fires_a, 1);
        assert_eq!(s.triggered_fires_b, 0);
    }

    #[test]
    fn unblocked_attack_can_cause_deckout_win() {
        // Defender has only 1 card left in deck; 1-power attack mills it → defender loses.
        let mut s = GameState::new(deck_of(50, "a"), deck_of(6, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);
        s.declare_attacker(&atk).unwrap();
        s.confirm_attacks().unwrap();
        let outcome = s.confirm_blocks().unwrap();
        assert_eq!(outcome.defender_milled_to_exile, 1);
        assert_eq!(s.winner, Some(PlayerId::A));
    }
}
