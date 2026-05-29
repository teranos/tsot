//! GameState — the data model for a tsot game in progress.
//!
//! Mirrors RULES.md sections F, U, L, S, Z, T, C, P, V.

use crate::card::Card;
use std::collections::HashMap;

/// F.2: exactly two players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerId {
    A,
    B,
}

impl PlayerId {
    pub fn opponent(self) -> PlayerId {
        match self {
            PlayerId::A => PlayerId::B,
            PlayerId::B => PlayerId::A,
        }
    }
}

/// U.6: phases in canonical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Untap,
    Draw,
    Main1,
    Combat,
    Main2,
    End,
}

/// Z.1–Z.5: per-player zones. Z.6 (ATTACHED) is encoded as a child list under each on-board instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Board,
    Deck,
    Hand,
    Graveyard,
    Exile,
}

pub type InstanceId = String;

/// A specific copy of a card in the game.
#[derive(Debug, Clone)]
pub struct CardInstance {
    pub instance_id: InstanceId,
    pub card: Card,
    pub owner: PlayerId,            // T.2 — immutable
    pub controller: PlayerId,       // T.1 — defaults to owner; effects may change it
    pub tapped: bool,               // B.4
    pub face_down: bool,            // P.17 (for attached)
    pub damage: i32,                // B.7–B.8 accumulated
    pub attached: Vec<InstanceId>,  // Z.6
    pub modifiers: Vec<Modifier>,   // C.12 continuous effects
    pub status_effects: Vec<StatusEffect>,
}

/// Continuous modifiers applied to a card's effective state.
#[derive(Debug, Clone)]
pub enum Modifier {
    /// e.g., +1/+1
    StatBoost { x: i32, y: i32 },
    /// e.g., Companion Bird grants flying while attached
    GainsFlying,
    /// e.g., Flesh-eating Plant
    CantAttack,
}

/// Status effects with bounded duration.
#[derive(Debug, Clone)]
pub enum StatusEffect {
    /// Stinging-bee: skip the next N untap steps
    SkipUntap(u32),
}

/// Per-player state. Zones reference instances by ID; the canonical
/// CardInstance lives in GameState.card_pool.
#[derive(Debug, Clone, Default)]
pub struct PlayerState {
    pub board: Vec<InstanceId>,
    pub hand: Vec<InstanceId>,
    pub deck: Vec<InstanceId>,        // first element = top of deck (V.1)
    pub graveyard: Vec<InstanceId>,
    pub exile: Vec<InstanceId>,
}

/// The full game state.
#[derive(Debug, Clone)]
pub struct GameState {
    pub a: PlayerState,
    pub b: PlayerState,
    pub card_pool: HashMap<InstanceId, CardInstance>,
    pub active_player: PlayerId,
    pub turn: u32,
    pub phase: Phase,
    pub winner: Option<PlayerId>,
}

impl GameState {
    /// S.1: each player starts with 5 cards in hand.
    /// Does not yet implement S.2/S.3 mulligan — those require player input.
    /// Cards passed in are dealt in order: first 5 become HAND, rest become DECK.
    /// Real games will shuffle the deck before this call; this function does not.
    pub fn new(deck_a: Vec<Card>, deck_b: Vec<Card>) -> Self {
        let mut card_pool = HashMap::new();
        let a = Self::init_player(PlayerId::A, deck_a, &mut card_pool);
        let b = Self::init_player(PlayerId::B, deck_b, &mut card_pool);

        GameState {
            a,
            b,
            card_pool,
            active_player: PlayerId::A,
            turn: 1,
            phase: Phase::Untap,
            winner: None,
        }
    }

    fn init_player(
        pid: PlayerId,
        cards: Vec<Card>,
        pool: &mut HashMap<InstanceId, CardInstance>,
    ) -> PlayerState {
        let mut state = PlayerState::default();
        let mut ids: Vec<InstanceId> = Vec::with_capacity(cards.len());
        for (i, card) in cards.into_iter().enumerate() {
            let iid = format!("{:?}:{:04}:{}", pid, i, card.id);
            let inst = CardInstance {
                instance_id: iid.clone(),
                card,
                owner: pid,
                controller: pid,
                tapped: false,
                face_down: false,
                damage: 0,
                attached: Vec::new(),
                modifiers: Vec::new(),
                status_effects: Vec::new(),
            };
            pool.insert(iid.clone(), inst);
            ids.push(iid);
        }
        // S.1: top 5 dealt to hand, rest stay as deck (in original order).
        let split_at = 5.min(ids.len());
        let deck = ids.split_off(split_at);
        state.hand = ids;
        state.deck = deck;
        state
    }

    /// Borrow a player by id.
    pub fn player(&self, id: PlayerId) -> &PlayerState {
        match id {
            PlayerId::A => &self.a,
            PlayerId::B => &self.b,
        }
    }

    /// Borrow a player mutably by id.
    pub fn player_mut(&mut self, id: PlayerId) -> &mut PlayerState {
        match id {
            PlayerId::A => &mut self.a,
            PlayerId::B => &mut self.b,
        }
    }

    /// L.1: a player loses when their DECK has no cards left.
    /// Returns the loser's id, if any. Caller should set self.winner to the opponent.
    pub fn check_loss(&self) -> Option<PlayerId> {
        if self.a.deck.is_empty() {
            return Some(PlayerId::A);
        }
        if self.b.deck.is_empty() {
            return Some(PlayerId::B);
        }
        None
    }

    /// C.12: effective stats = printed X/Y + sum of active stat modifiers.
    /// Re-evaluated on every call (no caching).
    /// Returns (0, 0) for cards without printed stats (instants, spells, artifacts).
    pub fn effective_stats(&self, iid: &InstanceId) -> (i32, i32) {
        let Some(inst) = self.card_pool.get(iid) else {
            return (0, 0);
        };
        let (mut x, mut y) = inst.card.stats.map(|s| (s.x, s.y)).unwrap_or((0, 0));
        for m in &inst.modifiers {
            if let Modifier::StatBoost { x: dx, y: dy } = m {
                x += dx;
                y += dy;
            }
        }
        (x, y)
    }

    /// Advance to the next phase, firing the entry action for the new phase.
    /// Phase order per U.6: Untap, Draw, Main1, Combat, Main2, End → next turn's Untap.
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

    /// B.10: end of turn clears all accumulated damage on creatures.
    fn clear_all_damage(&mut self) {
        for inst in self.card_pool.values_mut() {
            inst.damage = 0;
        }
    }

    /// Move a card between two zones owned by a single player.
    /// Returns Err if the instance isn't found in the source zone.
    pub fn move_card(
        &mut self,
        iid: &InstanceId,
        side: PlayerId,
        from: Zone,
        to: Zone,
    ) -> Result<(), MoveError> {
        let p = self.player_mut(side);
        let src = match from {
            Zone::Board => &mut p.board,
            Zone::Deck => &mut p.deck,
            Zone::Hand => &mut p.hand,
            Zone::Graveyard => &mut p.graveyard,
            Zone::Exile => &mut p.exile,
        };
        let pos = src
            .iter()
            .position(|x| x == iid)
            .ok_or(MoveError::NotInZone)?;
        src.remove(pos);

        let dst = match to {
            Zone::Board => &mut p.board,
            Zone::Deck => &mut p.deck,
            Zone::Hand => &mut p.hand,
            Zone::Graveyard => &mut p.graveyard,
            Zone::Exile => &mut p.exile,
        };
        dst.push(iid.clone());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    NotInZone,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Card, CardType, Stats};

    fn card_with_stats(id: &str, x: i32, y: i32) -> Card {
        Card {
            id: id.to_string(),
            name: String::new(),
            colors: vec![],
            kind: CardType::Creature,
            subtypes: vec![],
            symbol: String::new(),
            cost: vec![],
            abilities: vec![],
            stats: Some(Stats { x, y }),
        }
    }

    fn card_no_stats(id: &str) -> Card {
        Card {
            id: id.to_string(),
            name: String::new(),
            colors: vec![],
            kind: CardType::Instant,
            subtypes: vec![],
            symbol: String::new(),
            cost: vec![],
            abilities: vec![],
            stats: None,
        }
    }

    fn deck_of(n: usize, prefix: &str) -> Vec<Card> {
        (0..n)
            .map(|i| card_with_stats(&format!("{prefix}-{i}"), 1, 1))
            .collect()
    }

    #[test]
    fn new_game_deals_5_to_hand() {
        let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        assert_eq!(s.a.hand.len(), 5);
        assert_eq!(s.a.deck.len(), 45);
        assert_eq!(s.b.hand.len(), 5);
        assert_eq!(s.b.deck.len(), 45);
    }

    #[test]
    fn new_game_initial_state() {
        let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        assert_eq!(s.active_player, PlayerId::A);
        assert_eq!(s.phase, Phase::Untap);
        assert_eq!(s.turn, 1);
        assert!(s.winner.is_none());
    }

    #[test]
    fn new_game_card_pool_has_all_instances() {
        let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        assert_eq!(s.card_pool.len(), 100);
    }

    #[test]
    fn instances_carry_owner_and_controller() {
        let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        for iid in s.a.hand.iter().chain(s.a.deck.iter()) {
            let inst = s.card_pool.get(iid).unwrap();
            assert_eq!(inst.owner, PlayerId::A);
            assert_eq!(inst.controller, PlayerId::A);
        }
        for iid in s.b.hand.iter().chain(s.b.deck.iter()) {
            let inst = s.card_pool.get(iid).unwrap();
            assert_eq!(inst.owner, PlayerId::B);
            assert_eq!(inst.controller, PlayerId::B);
        }
    }

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
    fn check_loss_detects_empty_deck() {
        let s = GameState::new(deck_of(5, "a"), deck_of(50, "b"));
        assert_eq!(s.check_loss(), Some(PlayerId::A));
        let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        assert_eq!(s.check_loss(), None);
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

        // Advance to A's turn 3 Untap (2 full turn cycles).
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

        // Advance to B's turn 2 Untap (one full A turn).
        for _ in 0..6 {
            s.next_phase();
        }
        assert_eq!(s.phase, Phase::Untap);
        assert_eq!(s.active_player, PlayerId::B);
        // A's card should still be tapped — B's untap doesn't affect A's board.
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

        // Advance 12 phases — A's next Untap fires, SkipUntap(2) → (1), still tapped.
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

        // Another 12 phases — A's next Untap, SkipUntap(1) → removed, still tapped.
        for _ in 0..12 {
            s.next_phase();
        }
        let inst = s.card_pool.get(&iid).unwrap();
        assert!(inst.tapped);
        assert!(inst.status_effects.is_empty());

        // Another 12 phases — A's next Untap, no skip, card untaps for real.
        for _ in 0..12 {
            s.next_phase();
        }
        assert!(!s.card_pool.get(&iid).unwrap().tapped);
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

    #[test]
    fn effective_stats_returns_printed_without_modifiers() {
        let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = &s.a.hand[0];
        assert_eq!(s.effective_stats(iid), (1, 1));
    }

    #[test]
    fn effective_stats_sums_stat_boost_modifiers() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.modifiers.push(Modifier::StatBoost { x: 1, y: 0 });
        inst.modifiers.push(Modifier::StatBoost { x: 2, y: 2 });
        inst.modifiers.push(Modifier::StatBoost { x: -1, y: 1 });
        assert_eq!(s.effective_stats(&iid), (3, 4));
    }

    #[test]
    fn effective_stats_returns_zero_for_card_without_printed_stats() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        s.card_pool.get_mut(&iid).unwrap().card = card_no_stats("instant");
        assert_eq!(s.effective_stats(&iid), (0, 0));
    }

    #[test]
    fn move_card_succeeds_when_present_in_source() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        assert!(s
            .move_card(&iid, PlayerId::A, Zone::Hand, Zone::Graveyard)
            .is_ok());
        assert!(!s.a.hand.contains(&iid));
        assert!(s.a.graveyard.contains(&iid));
    }

    #[test]
    fn move_card_errors_when_not_in_source() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        assert_eq!(
            s.move_card(&iid, PlayerId::A, Zone::Graveyard, Zone::Exile),
            Err(MoveError::NotInZone)
        );
    }

    #[test]
    fn player_id_opponent_swaps() {
        assert_eq!(PlayerId::A.opponent(), PlayerId::B);
        assert_eq!(PlayerId::B.opponent(), PlayerId::A);
    }
}
