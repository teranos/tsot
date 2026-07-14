//! Turn-flow methods on GameState.
//!
//! Mirrors RULES.md sections U (turns) and B.10 (end-of-turn damage clear).

use super::context::EventContext;
use super::lua_api;
use super::state::{GameState, InstanceId, Phase, StatusEffect, Zone};
use crate::card::EventName;

/// Outcome shape for `next_phase`. Mirrors `PlayError::ChoicePending` /
/// `CombatError::ChoicePending` / `ActivateError::ChoicePending`: when
/// a Lua handler fired during a phase-advance trigger (currently
/// OnTurnBegin at the End→Untap transition) calls `game.choose_card` /
/// `game.confirm` / `game.choose_player` / `game.choose_int` against
/// an oracle that needs the human to answer, the wrapper raises
/// `Error::external(_)`, `fire_self_only` downcasts to
/// `Result<(), ChoicePending>`, and `next_phase` lifts via `?`. The
/// StepEngine catches this variant, rolls back the preview journal,
/// surfaces a `HumanPrompt::Choose*`, and re-fires the operation after
/// the user's answer is appended to `HumanReplayOracle.replay`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnError {
    ChoicePending(crate::choice::ChoicePending),
}

impl GameState {
    /// Advance to the next phase, firing the entry action for the new phase.
    /// Phase order per U.6: Untap → Draw → Main1 → Combat → Main2 → End → next turn's Untap.
    /// No-op if a winner has been determined. `ctx` is required for any
    /// phase-entry events that need to fire Lua handlers
    /// (`OnTurnBegin` at Untap entry currently); pass `None` from sites
    /// that don't have a Lua VM in scope.
    pub fn next_phase(&mut self, ctx: Option<&mut EventContext>) -> Result<(), TurnError> {
        if self.winner.is_some() {
            return Ok(());
        }
        let from = self.phase;
        let next = match self.phase {
            Phase::Untap => Phase::Draw,
            Phase::Draw => Phase::Main1,
            Phase::Main1 => Phase::Combat,
            Phase::Combat => Phase::Main2,
            Phase::Main2 => Phase::End,
            Phase::End => {
                // Transition into the next turn.
                self.clear_all_damage();
                self.clear_eot_modifiers();
                self.set_creature_attacked_this_turn(false);
                self.clear_all_attacked_this_turn();
                // P.35: clear the per-player Symbol-cast cap so each
                // player gets one cast per their own turn. Both
                // players reset; only the active player can ever set
                // the flag, but the inactive side's prior-turn value
                // would carry forward if we only cleared one side.
                self.set_symbol_cast_this_turn(super::state::PlayerId::A, false);
                self.set_symbol_cast_this_turn(super::state::PlayerId::B, false);
                // Extra-turn queue (azure-recursion etc.): if non-empty,
                // the front entry becomes the next active player instead
                // of the default opponent swap.
                let next_active = if !self.extra_turns_pending.is_empty() {
                    self.extra_turns_pending.remove(0)
                } else {
                    self.active_player.opponent()
                };
                self.set_active_player(next_active);
                let new_turn = self.turn + 1;
                self.set_turn(new_turn);
                Phase::Untap
            }
        };
        self.set_phase(next);
        // Cryogenic Chamber-style "return at next main phase": at the
        // start of either Main1 or Main2 (whichever begins first after
        // a schedule_return_at_next_main call), flush every queued
        // iid from wherever it currently sits back to its owner's
        // BOARD. Idempotent — the queue clears after flushing.
        if matches!(next, Phase::Main1 | Phase::Main2)
            && !self.pending_main_phase_returns.is_empty()
        {
            use super::state::{PlayerId, Zone};
            let queue = std::mem::take(&mut self.pending_main_phase_returns);
            for iid in queue {
                let Some(inst) = self.card_pool.get(&iid) else { continue; };
                let owner = inst.owner;
                // Remove from the non-board zone where the card lives.
                // The Chamber's flow puts it in EXILE (via P.8 cascade
                // after on_die), but we cover Hand / Deck / Graveyard
                // too so any future scheduler shape keeps working.
                let zones = [Zone::Exile, Zone::Graveyard, Zone::Hand, Zone::Deck];
                for pid in [PlayerId::A, PlayerId::B] {
                    for zone in zones {
                        let present = match zone {
                            Zone::Hand => self.player(pid).hand.contains(&iid),
                            Zone::Deck => self.player(pid).deck.contains(&iid),
                            Zone::Graveyard => self.player(pid).graveyard.contains(&iid),
                            Zone::Exile => self.player(pid).exile.contains(&iid),
                            Zone::Board => false,
                        };
                        if present {
                            self.remove_from_zone(&iid, pid, zone);
                        }
                    }
                }
                self.player_mut(owner).board.push(iid);
            }
        }
        // O2: Phase advance event. Emitted regardless of whether the
        // phase value actually differs (the End → Untap transition
        // also flips turn counter, which is a useful observation
        // even though both phases are upper-bound bookkeeping). The
        // bus push is a no-op when trace is disabled.
        if crate::trace::is_enabled() {
            crate::trace::push(crate::trace::TraceEvent::Phase {
                at_us: crate::trace::now_us(),
                turn: self.turn,
                from,
                to: next,
            });
        }
        // OnTurnBegin: fires when entering Untap (start of a new turn).
        // Broadcasts to every BOARD card of the active player plus
        // every card attached to one of those cards.
        if matches!(next, Phase::Untap) {
            if let Some(c) = ctx {
                let board: Vec<InstanceId> = self.player(self.active_player).board.clone();
                for iid in &board {
                    // Z.7: fused same-sleeve mutations get their phase-entry
                    // handlers (on_upkeep / on_untap_step) fired too.
                    let attached: Vec<InstanceId> = self
                        .card_pool
                        .get(iid)
                        .map(|i| i.children().cloned().collect())
                        .unwrap_or_default();
                    lua_api::fire_self_only(
                        c.lua,
                        self,
                        c.oracle(),
                        EventName::OnTurnBegin,
                        iid,
                    )
                    .map_err(TurnError::ChoicePending)?;
                    for aid in &attached {
                        lua_api::fire_self_only(
                            c.lua,
                            self,
                            c.oracle(),
                            EventName::OnTurnBegin,
                            aid,
                        )
                        .map_err(TurnError::ChoicePending)?;
                    }
                }
            }
        }
        self.enter_phase_action();
        Ok(())
    }

    fn enter_phase_action(&mut self) {
        match self.phase {
            Phase::Untap => self.do_untap_step(),
            Phase::Draw => self.do_draw_step(),
            Phase::End => self.do_end_step(),
            // TODO(events): each phase entry should fire phase-begin triggers per A.1.
            // E.g., "at the beginning of your upkeep / draw step / combat / end step".
            // Also: end of turn must fire end-of-turn triggers (e.g., delayed effects
            // queued by attach-shuffler's "return this creature to your hand at end of turn").
            _ => {}
        }
    }

    /// U.2: at the beginning of a player's turn, tapped cards untap.
    /// Cards with a SkipUntap status effect skip this untap and decrement their counter.
    fn do_untap_step(&mut self) {
        let pid = self.active_player;
        let board_ids: Vec<InstanceId> = self.player(pid).board.clone();
        for iid in board_ids {
            let (skip_pos, skip_n) = match self.card_pool.get(&iid) {
                Some(inst) => {
                    let pos = inst
                        .status_effects
                        .iter()
                        .position(|s| matches!(s, StatusEffect::SkipUntap(_)));
                    let n = pos.map(|p| match inst.status_effects[p] {
                        StatusEffect::SkipUntap(n) => n,
                    });
                    (pos, n)
                }
                None => continue,
            };
            if let (Some(pos), Some(n)) = (skip_pos, skip_n) {
                let mut new_effects = self.card_pool.get(&iid).unwrap().status_effects.clone();
                if n <= 1 {
                    new_effects.remove(pos);
                } else {
                    new_effects[pos] = StatusEffect::SkipUntap(n - 1);
                }
                self.set_status_effects(&iid, new_effects);
            } else {
                self.set_tapped(&iid, false);
            }
            // B.3 sickness clears at the start of controller's turn.
            self.set_summoning_sick(&iid, false);
        }
    }

    /// U.3 + U.4: active player draws 1 card from the top of their DECK.
    /// L.1: if the DECK is empty, the active player loses immediately.
    fn do_draw_step(&mut self) {
        let pid = self.active_player;
        if !self.draw_one(pid) {
            self.set_winner(Some(pid.opponent()), "deckout_draw");
        }
    }

    /// Z.8b: draw one CARD. A cardless sleeve (Z.8) on top of the deck does
    /// not satisfy the draw — it is collected into HAND for free and the
    /// draw continues, cascading through consecutive empties, until the
    /// first card-bearing sleeve is drawn into HAND. Returns `true` if a
    /// card was drawn, `false` if the deck emptied first (the caller
    /// resolves the deckout per L.1). All moves go through the journaled
    /// `move_card_or_emit`, so this rolls back cleanly.
    pub(crate) fn draw_one(&mut self, pid: super::state::PlayerId) -> bool {
        loop {
            let Some(top) = self.player(pid).deck.first().cloned() else {
                return false; // deck empty — no card drawn
            };
            let cardless = self
                .card_pool
                .get(&top)
                .map(|s| s.is_cardless())
                .unwrap_or(false);
            // Sacred-error sweep: deck-top → hand.
            let _ = self.move_card_or_emit(&top, pid, Zone::Deck, Zone::Hand, "draw-z8b");
            if !cardless {
                return true; // drew a card-bearing sleeve
            }
            // Cardless sleeve collected for free; keep drawing.
        }
    }

    /// U.10: at End phase, the active player discards down to a HAND size of 6.
    /// Discarded cards go to GRAVEYARD.
    fn do_end_step(&mut self) {
        const MAX_HAND: usize = 6;
        let pid = self.active_player;
        let hand_len = self.player(pid).hand.len();
        if hand_len <= MAX_HAND {
            return;
        }
        let to_discard = hand_len - MAX_HAND;
        for _ in 0..to_discard {
            let Some(front) = self.player(pid).hand.first().cloned() else {
                break;
            };
            // Sacred-error sweep: end-step discard hand-front → graveyard.
            let _ = self.move_card_or_emit(
                &front,
                pid,
                Zone::Hand,
                Zone::Graveyard,
                "turn-end-discard",
            );
            self.bump_action("discard", pid);
        }
    }

    /// B.10: end of turn clears all accumulated damage on creatures.
    fn clear_all_damage(&mut self) {
        let iids: Vec<InstanceId> = self.card_pool.keys().cloned().collect();
        for iid in &iids {
            self.set_damage(iid, 0.0);
        }
    }

    /// Companion to `set_creature_attacked_this_turn(false)` — clears the
    /// per-instance flag that drives "did THIS creature attack this turn"
    /// conditions in activated abilities (vigilant-human, et al.).
    fn clear_all_attacked_this_turn(&mut self) {
        let iids: Vec<InstanceId> = self.card_pool.keys().cloned().collect();
        for iid in &iids {
            self.set_attacked_this_turn(iid, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::{Phase, PlayerId, StatusEffect};
    use super::*;
    use crate::game::test_helpers::*;

    #[test]
    fn phase_cycles_in_order() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        assert_eq!(s.phase, Phase::Untap);
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.phase, Phase::Draw);
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.phase, Phase::Main1);
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.phase, Phase::Combat);
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.phase, Phase::Main2);
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.phase, Phase::End);
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.phase, Phase::Untap);
    }

    #[test]
    fn end_to_untap_swaps_active_and_increments_turn() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        for _ in 0..6 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.phase, Phase::Untap);
        assert_eq!(s.turn, 2);
        assert_eq!(s.active_player, PlayerId::B);
    }

    #[test]
    fn draw_step_moves_top_of_deck_to_hand() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let hand_before = s.a.hand.len();
        let deck_before = s.a.deck.len();
        let top = s.a.deck[0].clone();
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.phase, Phase::Draw);
        assert_eq!(s.a.hand.len(), hand_before + 1);
        assert_eq!(s.a.deck.len(), deck_before - 1);
        assert_eq!(s.a.hand.last(), Some(&top));
    }

    #[test]
    fn empty_deck_on_a_draw_makes_b_winner() {
        let mut s = GameState::new(deck_of(5, "a"), deck_of(50, "b"));
        assert_eq!(s.a.deck.len(), 0);
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.winner, Some(PlayerId::B));
    }

    #[test]
    fn empty_deck_on_b_draw_makes_a_winner() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(5, "b"));
        for _ in 0..7 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.winner, Some(PlayerId::A));
    }

    #[test]
    fn winner_set_makes_next_phase_a_noop() {
        let mut s = GameState::new(deck_of(5, "a"), deck_of(50, "b"));
        s.next_phase(None).expect("None ctx never yields");
        assert!(s.winner.is_some());
        let phase_before = s.phase;
        let turn_before = s.turn;
        s.next_phase(None).expect("None ctx never yields");
        assert_eq!(s.phase, phase_before);
        assert_eq!(s.turn, turn_before);
    }

    #[test]
    fn turn_subsystem_round_trips_through_journal() {
        // Open journal, run several phases including untap/draw/end/clear,
        // rollback, assert state byte-identical.
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        // Place a creature on A's board with damage + skip_untap, so the
        // untap step has meaningful mutations to capture.
        let iid = s.a.hand[0].clone();
        s.a.hand.remove(0);
        s.a.board.push(iid.clone());
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.tapped = true;
        inst.damage = 3.0;
        inst.status_effects.push(StatusEffect::SkipUntap(2));

        let snapshot = format!("{:?}", s);
        s.journal = Some(crate::game::Journal::new());

        // Advance through a full cycle: Untap → Draw → ... → End → Untap.
        for _ in 0..6 {
            s.next_phase(None).expect("None ctx never yields");
        }

        assert_ne!(snapshot, format!("{:?}", s));
        let journal = s.journal.take().unwrap();
        journal.rollback(&mut s);
        assert!(s.journal.is_none());
        assert_eq!(
            snapshot,
            format!("{:?}", s),
            "turn subsystem rollback should restore prior state"
        );
    }

    #[test]
    fn natural_deckout_takes_91_turns_with_50_card_decks() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        while s.winner.is_none() {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.winner, Some(PlayerId::B));
        assert_eq!(s.turn, 91);
    }

    #[test]
    fn untap_step_untaps_active_players_creatures() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        s.a.hand.remove(0);
        s.a.board.push(iid.clone());
        s.card_pool.get_mut(&iid).unwrap().tapped = true;

        for _ in 0..12 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.phase, Phase::Untap);
        assert_eq!(s.active_player, PlayerId::A);
        assert!(!s.card_pool.get(&iid).unwrap().tapped);
    }

    #[test]
    fn untap_step_only_untaps_active_player_cards() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        s.a.hand.remove(0);
        s.a.board.push(iid.clone());
        s.card_pool.get_mut(&iid).unwrap().tapped = true;

        for _ in 0..6 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.phase, Phase::Untap);
        assert_eq!(s.active_player, PlayerId::B);
        assert!(s.card_pool.get(&iid).unwrap().tapped);
    }

    #[test]
    fn skip_untap_status_decrements_and_keeps_tapped() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        s.a.hand.remove(0);
        s.a.board.push(iid.clone());
        {
            let inst = s.card_pool.get_mut(&iid).unwrap();
            inst.tapped = true;
            inst.status_effects.push(StatusEffect::SkipUntap(2));
        }

        for _ in 0..12 {
            s.next_phase(None).expect("None ctx never yields");
        }
        let inst = s.card_pool.get(&iid).unwrap();
        assert!(inst.tapped);
        assert_eq!(inst.status_effects.len(), 1);
        assert!(matches!(
            inst.status_effects[0],
            StatusEffect::SkipUntap(1)
        ));

        for _ in 0..12 {
            s.next_phase(None).expect("None ctx never yields");
        }
        let inst = s.card_pool.get(&iid).unwrap();
        assert!(inst.tapped);
        assert!(inst.status_effects.is_empty());

        for _ in 0..12 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert!(!s.card_pool.get(&iid).unwrap().tapped);
    }

    #[test]
    fn end_phase_discards_active_player_down_to_six() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        // Stuff 9 cards into A's hand (5 default + 4 manually moved from deck).
        for _ in 0..4 {
            let top = s.a.deck.remove(0);
            s.a.hand.push(top);
        }
        assert_eq!(s.a.hand.len(), 9);
        let oldest_three: Vec<_> = s.a.hand[0..3].to_vec();
        let gy_before = s.a.graveyard.len();

        // Advance to End: Untap → Draw → Main1 → Combat → Main2 → End.
        // Drawing adds one more card, making the hand 10 before discard.
        for _ in 0..5 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.phase, Phase::End);
        assert_eq!(s.a.hand.len(), 6);
        // The first three (oldest) should be gone, in graveyard.
        for iid in &oldest_three {
            assert!(s.a.graveyard.contains(iid), "expected {iid} in graveyard");
            assert!(!s.a.hand.contains(iid), "expected {iid} out of hand");
        }
        assert_eq!(s.a.graveyard.len(), gy_before + 4); // 9 + 1 drawn - 6 = 4 discarded
    }

    #[test]
    fn end_phase_does_nothing_when_hand_at_or_below_six() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        // Default hand size is 5; advance to End. Draw makes it 6, exactly the cap.
        for _ in 0..5 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.phase, Phase::End);
        assert_eq!(s.a.hand.len(), 6);
        assert!(s.a.graveyard.is_empty());
    }

    #[test]
    fn end_phase_only_discards_active_player() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        // Stuff B's hand to 9 (the inactive player on A's turn).
        for _ in 0..4 {
            let top = s.b.deck.remove(0);
            s.b.hand.push(top);
        }
        assert_eq!(s.b.hand.len(), 9);
        for _ in 0..5 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.phase, Phase::End);
        assert_eq!(s.active_player, PlayerId::A);
        // B is inactive, hand untouched.
        assert_eq!(s.b.hand.len(), 9);
        assert!(s.b.graveyard.is_empty());
    }

    #[test]
    fn end_of_turn_clears_damage() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        s.card_pool.get_mut(&iid).unwrap().damage = 5.0;
        for _ in 0..6 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.card_pool.get(&iid).unwrap().damage, 0.0);
    }

    /// Mirror of the activate.rs / combat_tests.rs propagation tests at
    /// the turn boundary. Pins the contract that when an OnTurnBegin
    /// handler (fired during the End→Untap transition inside
    /// `next_phase`) calls `game.choose_card` against a
    /// HumanReplayOracle with no replay, `next_phase` returns
    /// `Err(TurnError::ChoicePending(_))` instead of silently
    /// swallowing the suspend. Closes the last two `let _ =` discard
    /// sites flagged in LIMITATIONS.md.
    #[test]
    fn next_phase_returns_choice_pending_when_on_turn_begin_yields() {
        use crate::card::EventName;
        use crate::choice::{ChoicePending, RandomOracle};
        use crate::game::context::EventContext;
        use crate::sim::human::HumanReplayOracle;
        use mlua::Lua;
        use rand::SeedableRng;

        let lua = Lua::new();
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        // Park a card on B's board: End→Untap swaps active to B, so
        // the OnTurnBegin sweep iterates B's board.
        let iid = s.b.hand[0].clone();
        s.b.hand.remove(0);
        s.b.board.push(iid.clone());
        // Pool element for choose_card; the wrapper suspends before
        // reading it, but it has to be non-empty for the request.
        let target = s.a.hand[0].clone();

        let handler_src = format!(
            r#"return function(game, self)
                 local picked = game.choose_card({{ "{target}" }}, {{ prompt = "test" }})
                 if picked ~= nil then game.damage(picked, 1) end
               end"#
        );
        let handler: mlua::Function = lua.load(&handler_src).eval().unwrap();
        s.card_pool
            .get_mut(&iid)
            .unwrap()
            .card_mut()
            .handlers
            .insert(EventName::OnTurnBegin, handler);

        // Advance to End (Untap → Draw → Main1 → Combat → Main2 → End
        // is 5 transitions). The 6th transition is End→Untap with
        // active swapping to B and OnTurnBegin firing.
        for _ in 0..5 {
            s.next_phase(None).expect("None ctx never yields");
        }
        assert_eq!(s.phase, Phase::End);

        let mut oracle = HumanReplayOracle::new(
            RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0)),
            Some(PlayerId::B),
        );

        let result = {
            let mut ctx = EventContext::new(&lua, &mut oracle);
            s.next_phase(Some(&mut ctx))
        };

        match result {
            Err(TurnError::ChoicePending(ChoicePending::Card(req))) => {
                assert_eq!(req.asker, Some(PlayerId::B));
                assert!(!req.pool.is_empty());
            }
            other => panic!(
                "expected Err(TurnError::ChoicePending(Card)), got {other:?}"
            ),
        }
    }
}
