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
use crate::card::EventName;
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
    ///
    /// When `lua` is `Some`, the attacker's `on_attack` handler fires after the
    /// attack is recorded.
    pub fn declare_attacker(
        &mut self,
        attacker: &InstanceId,
        lua: Option<&Lua>,
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
            None => self.combat = Some(CombatState::AwaitingAttackers),
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

        // Append to the buffered attacker list. The buffer is encoded as
        // AwaitingBlockers from the start (data shape matches); confirm_attacks
        // is the no-op marker that says "no more attackers, blockers may now
        // declare." (The early-state-init block above already promoted None to
        // AwaitingAttackers; the first attacker transitions that to
        // AwaitingBlockers below.)
        match &mut self.combat {
            Some(CombatState::AwaitingAttackers) => {
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

        // LUA Phase 1: fire on_attack on the declared attacker.
        if let Some(lua) = lua {
            lua_api::fire_self_only(lua, self, EventName::OnAttack, attacker);
        }
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

        // LUA Phase 1: fire `on_blocked_by` on the attacker, then `on_block` on the blocker.
        // Per-blocker semantics for both. Order: attacker-side first, then blocker-side.
        // Errors log and continue per LUA.md Q #3.
        if let Some(lua) = lua {
            lua_api::fire_with_partner(lua, self, EventName::OnBlockedBy, attacker, blocker);
            lua_api::fire_with_partner(lua, self, EventName::OnBlock, blocker, attacker);
        }
        // TODO(stack): when block-declaration is added as an R.1 window-opener (per design
        // discussion), open a response window here.

        Ok(())
    }

    /// Finalize blockers and resolve combat: damage, deaths, B.2 mill.
    ///
    /// `lua` is the long-lived VM from `CardRegistry`. When `Some`, dying
    /// creatures' `on_die` handlers fire. When `None`, handlers are skipped.
    pub fn confirm_blocks(&mut self, lua: Option<&Lua>) -> Result<CombatOutcome, CombatError> {
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
        Ok(self.resolve_combat(attacks, lua))
    }

    fn resolve_combat(&mut self, attacks: Vec<AttackDecl>, lua: Option<&Lua>) -> CombatOutcome {
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
            // LUA Phase 1: fire on_die after the Board → Graveyard move so the
            // handler observes the post-death zone state. Handlers may return
            // attached cards via game.move; P.8 (auto-exile of leftover
            // attached) is still TODO and will run after handlers when wired.
            if let Some(lua) = lua {
                lua_api::fire_self_only(lua, self, EventName::OnDie, iid);
            }
            // TODO(types): P.8 — when a card with attached cards moves to GRAVEYARD,
            // any attached cards still present must move to EXILE.
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
        s.declare_attacker(&atk, None).unwrap();
        s.confirm_attacks().unwrap();
        let outcome = s.confirm_blocks(None).unwrap();
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

        s.declare_attacker(&atk, None).unwrap();
        s.confirm_attacks().unwrap();
        s.declare_blocker(&blk, &atk, None).unwrap();
        let outcome = s.confirm_blocks(None).unwrap();
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
        s.declare_attacker(&atk, None).unwrap();
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
        s.declare_attacker(&atk, None).unwrap();
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
            s.declare_attacker(&atk, None),
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
            s.declare_attacker(&atk, None),
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
        assert!(s.declare_attacker(&atk, None).is_ok());
    }

    #[test]
    fn tapped_creature_cannot_attack() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        s.card_pool.get_mut(&atk).unwrap().tapped = true;
        enter_combat(&mut s);
        assert_eq!(s.declare_attacker(&atk, None), Err(CombatError::AttackerTapped));
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
        s.declare_attacker(&atk, None).unwrap();
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
        s.declare_attacker(&atk, None).unwrap();
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
        s.declare_attacker(&atk, None).unwrap();
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
        s.declare_attacker(&atk, None).unwrap();
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
            s.declare_attacker(&atk, None),
            Err(CombatError::NotCombatPhase)
        );
    }

    #[test]
    fn battle_captain_untaps_other_attackers_on_attack() {
        use crate::card::CardRegistry;

        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let captain = registry
            .cards()
            .iter()
            .find(|c| c.id == "battle-captain")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let cap_iid = s.a.hand[0].clone();
        let other_iid = s.a.hand[1].clone();
        {
            let inst = s.card_pool.get_mut(&cap_iid).unwrap();
            inst.card.handlers = captain.handlers.clone();
            inst.card.id = captain.id.clone();
        }
        put_on_board(&mut s, PlayerId::A, &cap_iid);
        put_on_board(&mut s, PlayerId::A, &other_iid);
        add_ability(&mut s, &cap_iid, "haste");
        add_ability(&mut s, &other_iid, "haste");
        enter_combat(&mut s);

        // Other creature attacks first; it taps.
        s.declare_attacker(&other_iid, Some(registry.lua())).unwrap();
        assert!(s.card_pool.get(&other_iid).unwrap().tapped);

        // Captain attacks; its handler untaps the other attacker.
        s.declare_attacker(&cap_iid, Some(registry.lua())).unwrap();
        assert!(s.card_pool.get(&cap_iid).unwrap().tapped); // captain itself stays tapped
        assert!(!s.card_pool.get(&other_iid).unwrap().tapped);
    }

    fn registry_with_fixture(name: &str, source: &str) -> crate::card::CardRegistry {
        let tmp = std::env::temp_dir().join(format!("tsot_fixture_{name}"));
        std::fs::create_dir_all(&tmp).unwrap();
        // Clean any stale fixture from a prior run.
        if let Ok(rd) = std::fs::read_dir(&tmp) {
            for entry in rd.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let path = tmp.join(format!("{name}.lua"));
        std::fs::write(&path, source).unwrap();
        crate::card::CardRegistry::load(&tmp).unwrap()
    }

    #[test]
    fn on_attack_handler_fires_when_attacker_declared() {
        let registry = registry_with_fixture(
            "on_attack",
            r#"return {
                id = "fire-on-attack",
                on_attack = function(game, self)
                    _G.fire_on_attack_count = (_G.fire_on_attack_count or 0) + 1
                end,
            }"#,
        );
        let fixture = registry
            .cards()
            .iter()
            .find(|c| c.id == "fire-on-attack")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&atk).unwrap();
            inst.card.handlers = fixture.handlers.clone();
            inst.card.id = fixture.id.clone();
        }
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        registry
            .lua()
            .globals()
            .set("fire_on_attack_count", 0_i32)
            .unwrap();
        s.declare_attacker(&atk, Some(registry.lua())).unwrap();

        let count: i32 = registry
            .lua()
            .globals()
            .get("fire_on_attack_count")
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(s.event_fires[&crate::card::EventName::OnAttack], [1, 0]);
    }

    #[test]
    fn on_block_handler_fires_when_blocker_declared() {
        let registry = registry_with_fixture(
            "on_block",
            r#"return {
                id = "fire-on-block-side",
                on_block = function(game, self, attacker)
                    _G.fire_on_block_side_count = (_G.fire_on_block_side_count or 0) + 1
                end,
            }"#,
        );
        let fixture = registry
            .cards()
            .iter()
            .find(|c| c.id == "fire-on-block-side")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        // Handler goes on the BLOCKER, not the attacker.
        {
            let inst = s.card_pool.get_mut(&blk).unwrap();
            inst.card.handlers = fixture.handlers.clone();
            inst.card.id = fixture.id.clone();
        }
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        registry
            .lua()
            .globals()
            .set("fire_on_block_side_count", 0_i32)
            .unwrap();
        s.declare_attacker(&atk, None).unwrap();
        s.confirm_attacks().unwrap();
        s.declare_blocker(&blk, &atk, Some(registry.lua())).unwrap();

        let count: i32 = registry
            .lua()
            .globals()
            .get("fire_on_block_side_count")
            .unwrap();
        assert_eq!(count, 1);
        // Owner of blocker is B → credited to B.
        assert_eq!(s.event_fires[&crate::card::EventName::OnBlock], [0, 1]);
    }

    #[test]
    fn midnight_raven_attack_moves_top_of_deck_to_bottom() {
        use crate::card::CardRegistry;

        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let raven = registry
            .cards()
            .iter()
            .find(|c| c.id == "midnight-raven")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&atk).unwrap();
            inst.card.handlers = raven.handlers.clone();
            inst.card.id = raven.id.clone();
        }
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        let top_before = s.a.deck[0].clone();
        let bottom_before = s.a.deck.last().unwrap().clone();
        let deck_len = s.a.deck.len();

        s.declare_attacker(&atk, Some(registry.lua())).unwrap();

        // Top card moved to bottom; deck length unchanged.
        assert_eq!(s.a.deck.len(), deck_len);
        assert_eq!(s.a.deck.last().unwrap(), &top_before);
        // The card that *was* on the bottom is now one above the bottom.
        assert_eq!(s.a.deck[deck_len - 2], bottom_before);
        assert_eq!(s.event_fires[&crate::card::EventName::OnAttack], [1, 0]);
    }

    #[test]
    fn thorn_beetle_on_block_damages_attacker() {
        use crate::card::CardRegistry;

        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let beetle = registry
            .cards()
            .iter()
            .find(|c| c.id == "thorn-beetle")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let atk = s.a.hand[0].clone();
        let blk = s.b.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&blk).unwrap();
            inst.card.handlers = beetle.handlers.clone();
            inst.card.id = beetle.id.clone();
        }
        put_on_board(&mut s, PlayerId::A, &atk);
        put_on_board(&mut s, PlayerId::B, &blk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);

        s.declare_attacker(&atk, None).unwrap();
        s.confirm_attacks().unwrap();
        s.declare_blocker(&blk, &atk, Some(registry.lua())).unwrap();

        // Handler pinged the attacker for 1.
        assert_eq!(s.card_pool.get(&atk).unwrap().damage, 1);
        assert_eq!(s.event_fires[&crate::card::EventName::OnBlock], [0, 1]);
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

        s.declare_attacker(&atk, None).unwrap();
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

        s.declare_attacker(&atk, None).unwrap();
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

        s.declare_attacker(&atk, None).unwrap();
        s.confirm_attacks().unwrap();

        let a_hand_before = s.a.hand.len();
        let a_deck_before = s.a.deck.len();

        s.declare_blocker(&blk, &atk, Some(registry.lua())).unwrap();

        assert_eq!(s.a.hand.len(), a_hand_before + 1);
        assert_eq!(s.a.deck.len(), a_deck_before - 1);
        assert_eq!(s.total_fires(PlayerId::A), 1);
        assert_eq!(s.total_fires(PlayerId::B), 0);
        assert_eq!(s.event_fires[&crate::card::EventName::OnBlockedBy], [1, 0]);
    }

    #[test]
    fn trustworthy_lender_on_die_returns_attached_to_hand() {
        use crate::card::CardRegistry;

        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let lender = registry
            .cards()
            .iter()
            .find(|c| c.id == "trustworthy-lender")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let lender_iid = s.a.hand[0].clone();
        let attached_iid = s.a.hand[1].clone();
        let killer_iid = s.b.hand[0].clone();

        // Swap lender's card data in (keep stats so 1/1 vs 1/1 is mutual kill).
        {
            let inst = s.card_pool.get_mut(&lender_iid).unwrap();
            inst.card.handlers = lender.handlers.clone();
            inst.card.id = lender.id.clone();
        }

        put_on_board(&mut s, PlayerId::A, &lender_iid);
        put_on_board(&mut s, PlayerId::B, &killer_iid);
        // Attach the payment to lender (replicates what play_card would do).
        s.a.hand.retain(|x| x != &attached_iid);
        s.card_pool
            .get_mut(&lender_iid)
            .unwrap()
            .attached
            .push(attached_iid.clone());
        s.card_pool.get_mut(&attached_iid).unwrap().face_down = true;

        add_ability(&mut s, &lender_iid, "haste");
        enter_combat(&mut s);

        s.declare_attacker(&lender_iid, None).unwrap();
        s.confirm_attacks().unwrap();
        s.declare_blocker(&killer_iid, &lender_iid, None).unwrap();
        let outcome = s.confirm_blocks(Some(registry.lua())).unwrap();

        assert!(outcome.deaths.contains(&lender_iid));
        assert!(s.a.graveyard.contains(&lender_iid));

        // Handler returned attached to A's hand and flipped it face-up.
        assert!(s.a.hand.contains(&attached_iid));
        assert!(!s
            .card_pool
            .get(&lender_iid)
            .unwrap()
            .attached
            .contains(&attached_iid));
        assert!(!s.card_pool.get(&attached_iid).unwrap().face_down);
        assert_eq!(s.total_fires(PlayerId::A), 1);
        assert_eq!(s.event_fires[&crate::card::EventName::OnDie], [1, 0]);
    }

    #[test]
    fn unblocked_attack_can_cause_deckout_win() {
        // Defender has only 1 card left in deck; 1-power attack mills it → defender loses.
        let mut s = GameState::new(deck_of(50, "a"), deck_of(6, "b"));
        let atk = s.a.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &atk);
        add_ability(&mut s, &atk, "haste");
        enter_combat(&mut s);
        s.declare_attacker(&atk, None).unwrap();
        s.confirm_attacks().unwrap();
        let outcome = s.confirm_blocks(None).unwrap();
        assert_eq!(outcome.defender_milled_to_exile, 1);
        assert_eq!(s.winner, Some(PlayerId::A));
    }
}
