//! Turn-flow methods on GameState.
//!
//! Mirrors RULES.md sections U (turns) and B.10 (end-of-turn damage clear).

use super::state::{GameState, InstanceId, Phase, StatusEffect};

impl GameState {
    /// Advance to the next phase, firing the entry action for the new phase.
    /// Phase order per U.6: Untap → Draw → Main1 → Combat → Main2 → End → next turn's Untap.
    /// No-op if a winner has been determined.
    pub fn next_phase(&mut self) {
        if self.winner.is_some() {
            return;
        }
        let next = match self.phase {
            Phase::Untap => Phase::Draw,
            Phase::Draw => Phase::Main1,
            Phase::Main1 => Phase::Combat,
            Phase::Combat => Phase::Main2,
            Phase::Main2 => Phase::End,
            Phase::End => {
                // Transition into the next turn.
                self.clear_all_damage();
                self.active_player = self.active_player.opponent();
                self.turn += 1;
                Phase::Untap
            }
        };
        self.phase = next;
        self.enter_phase_action();
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
            let Some(inst) = self.card_pool.get_mut(&iid) else {
                continue;
            };
            let skip_pos = inst
                .status_effects
                .iter()
                .position(|s| matches!(s, StatusEffect::SkipUntap(_)));
            if let Some(pos) = skip_pos {
                let StatusEffect::SkipUntap(n) = inst.status_effects[pos];
                if n <= 1 {
                    inst.status_effects.remove(pos);
                } else {
                    inst.status_effects[pos] = StatusEffect::SkipUntap(n - 1);
                }
            } else {
                inst.tapped = false;
            }
            // B.3 sickness clears at the start of controller's turn.
            inst.summoning_sick = false;
        }
    }

    /// U.3 + U.4: active player draws 1 card from the top of their DECK.
    /// L.1: if the DECK is empty, the active player loses immediately.
    fn do_draw_step(&mut self) {
        let pid = self.active_player;
        if self.player(pid).deck.is_empty() {
            self.winner = Some(pid.opponent());
            return;
        }
        let p = self.player_mut(pid);
        let drawn = p.deck.remove(0);
        p.hand.push(drawn);
    }

    /// U.10: at End phase, the active player discards down to a HAND size of 6.
    /// Discarded cards go to GRAVEYARD.
    ///
    /// The rule says the active player chooses which cards to discard. Until the
    /// choice API lands (LUA Phase 2), this discards from the front of HAND
    /// (oldest-held first). Replace the `drain(0..to_discard)` line with a
    /// choice-driven selection when wiring `game.choose_card` for cleanup events.
    fn do_end_step(&mut self) {
        const MAX_HAND: usize = 6;
        let pid = self.active_player;
        let hand_len = self.player(pid).hand.len();
        if hand_len <= MAX_HAND {
            return;
        }
        let to_discard = hand_len - MAX_HAND;
        let p = self.player_mut(pid);
        let drained: Vec<InstanceId> = p.hand.drain(0..to_discard).collect();
        p.graveyard.extend(drained);
        for _ in 0..to_discard {
            self.bump_action("discard", pid);
        }
    }

    /// B.10: end of turn clears all accumulated damage on creatures.
    fn clear_all_damage(&mut self) {
        for inst in self.card_pool.values_mut() {
            inst.damage = 0;
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
        s.next_phase();
        assert_eq!(s.phase, Phase::Draw);
        s.next_phase();
        assert_eq!(s.phase, Phase::Main1);
        s.next_phase();
        assert_eq!(s.phase, Phase::Combat);
        s.next_phase();
        assert_eq!(s.phase, Phase::Main2);
        s.next_phase();
        assert_eq!(s.phase, Phase::End);
        s.next_phase();
        assert_eq!(s.phase, Phase::Untap);
    }

    #[test]
    fn end_to_untap_swaps_active_and_increments_turn() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        for _ in 0..6 {
            s.next_phase();
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
        s.next_phase();
        assert_eq!(s.phase, Phase::Draw);
        assert_eq!(s.a.hand.len(), hand_before + 1);
        assert_eq!(s.a.deck.len(), deck_before - 1);
        assert_eq!(s.a.hand.last(), Some(&top));
    }

    #[test]
    fn empty_deck_on_a_draw_makes_b_winner() {
        let mut s = GameState::new(deck_of(5, "a"), deck_of(50, "b"));
        assert_eq!(s.a.deck.len(), 0);
        s.next_phase();
        assert_eq!(s.winner, Some(PlayerId::B));
    }

    #[test]
    fn empty_deck_on_b_draw_makes_a_winner() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(5, "b"));
        for _ in 0..7 {
            s.next_phase();
        }
        assert_eq!(s.winner, Some(PlayerId::A));
    }

    #[test]
    fn winner_set_makes_next_phase_a_noop() {
        let mut s = GameState::new(deck_of(5, "a"), deck_of(50, "b"));
        s.next_phase();
        assert!(s.winner.is_some());
        let phase_before = s.phase;
        let turn_before = s.turn;
        s.next_phase();
        assert_eq!(s.phase, phase_before);
        assert_eq!(s.turn, turn_before);
    }

    #[test]
    fn natural_deckout_takes_91_turns_with_50_card_decks() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        while s.winner.is_none() {
            s.next_phase();
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
            s.next_phase();
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
            s.next_phase();
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
            s.next_phase();
        }
        let inst = s.card_pool.get(&iid).unwrap();
        assert!(inst.tapped);
        assert_eq!(inst.status_effects.len(), 1);
        assert!(matches!(
            inst.status_effects[0],
            StatusEffect::SkipUntap(1)
        ));

        for _ in 0..12 {
            s.next_phase();
        }
        let inst = s.card_pool.get(&iid).unwrap();
        assert!(inst.tapped);
        assert!(inst.status_effects.is_empty());

        for _ in 0..12 {
            s.next_phase();
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
            s.next_phase();
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
            s.next_phase();
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
            s.next_phase();
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
        s.card_pool.get_mut(&iid).unwrap().damage = 5;
        for _ in 0..6 {
            s.next_phase();
        }
        assert_eq!(s.card_pool.get(&iid).unwrap().damage, 0);
    }
}
