//! Core game-state types and basic accessors.
//!
//! Mirrors RULES.md sections F, U, L, S, Z, T, C.

use crate::card::{Card, EventName};
use std::collections::BTreeMap;

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
    pub owner: PlayerId,           // T.2 — immutable
    pub controller: PlayerId,      // T.1 — defaults to owner; effects may change it
    pub tapped: bool,              // B.4
    pub face_down: bool,           // P.17 (for attached)
    pub damage: i32,               // B.7–B.8 accumulated
    pub summoning_sick: bool,      // B.3 (cleared at start of controller's turn)
    pub attached: Vec<InstanceId>, // Z.6
    pub modifiers: Vec<Modifier>,  // C.12 continuous effects
    pub status_effects: Vec<StatusEffect>,
}

impl CardInstance {
    /// True if the card has the given (lowercase) keyword as one of its
    /// printed abilities, e.g. "flying", "haste", "vigilance", "defender", "unblockable".
    /// Also true if the card has a matching Modifier (Modifier::GainsFlying for "flying").
    pub fn has_keyword(&self, keyword: &str) -> bool {
        let printed = self.card.abilities.iter().any(|a| {
            let normalized = a.trim().trim_end_matches('.').to_lowercase();
            normalized == keyword
        });
        if printed {
            return true;
        }
        for m in &self.modifiers {
            if let Modifier::GainsFlying = m {
                if keyword == "flying" {
                    return true;
                }
            }
        }
        false
    }
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
    pub deck: Vec<InstanceId>, // first element = top of deck (V.1)
    pub graveyard: Vec<InstanceId>,
    pub exile: Vec<InstanceId>,
}

/// In-progress combat state during the Combat phase.
#[derive(Debug, Clone)]
pub enum CombatState {
    AwaitingAttackers,
    AwaitingBlockers { attacks: Vec<AttackDecl> },
}

/// One attacker and zero-or-more blockers assigned to it.
#[derive(Debug, Clone)]
pub struct AttackDecl {
    pub attacker: InstanceId,
    pub blockers: Vec<InstanceId>,
}

/// The full game state.
#[derive(Debug, Clone)]
pub struct GameState {
    pub a: PlayerState,
    pub b: PlayerState,
    pub card_pool: BTreeMap<InstanceId, CardInstance>,
    pub active_player: PlayerId,
    pub turn: u32,
    pub phase: Phase,
    pub winner: Option<PlayerId>,
    pub combat: Option<CombatState>,
    /// Engine metric: per-event handler-fire counts, credited to the owner of
    /// the source card. `[u32; 2]` indexed by player (0 = A, 1 = B). Diagnostic.
    pub event_fires: BTreeMap<EventName, [u32; 2]>,
    /// Engine metric: counts of each `game.*` action invoked from inside a
    /// handler. Keyed by short action name ("draw", "mill", "damage", "move").
    /// Player attribution depends on the action — see `bump_action` callers.
    pub action_counts: BTreeMap<&'static str, [u32; 2]>,
    /// Optional per-action mutation journal. `None` = no recording. Used for
    /// preview-and-rollback (sim's "would this play kill me?" check). When
    /// `Some`, every mutation pushes here instead of `replay_journal`.
    pub journal: Option<super::Journal>,
    /// Optional game-long mutation journal. Opened at game start by callers
    /// who want a complete replay. Helpers push here only when `journal` is
    /// `None` (so committed previews are merged in via `extend_from` and
    /// rolled-back ones leave it untouched).
    pub replay_journal: Option<super::Journal>,
}

impl GameState {
    /// S.1: each player starts with 5 cards in hand.
    /// Does not yet implement S.2/S.3 mulligan — those require player input.
    /// Cards passed in are dealt in order: first 5 become HAND, rest become DECK.
    /// Real games will shuffle the deck before this call; this function does not.
    pub fn new(deck_a: Vec<Card>, deck_b: Vec<Card>) -> Self {
        let mut card_pool = BTreeMap::new();
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
            combat: None,
            event_fires: BTreeMap::new(),
            action_counts: BTreeMap::new(),
            journal: None,
            replay_journal: None,
        }
    }

    /// Internal: returns whichever journal should receive the next mutation.
    /// Preview journal wins if open (so previews can be cleanly rolled back
    /// without polluting the replay journal). Falls back to replay journal,
    /// which accumulates only committed mutations.
    pub(crate) fn active_journal(&mut self) -> Option<&mut super::Journal> {
        if self.journal.is_some() {
            self.journal.as_mut()
        } else {
            self.replay_journal.as_mut()
        }
    }

    /// Engine helper: credit a successful handler fire to `owner` under `event`.
    pub fn bump_event_fire(&mut self, event: EventName, owner: PlayerId) {
        let entry = self.event_fires.entry(event).or_insert([0, 0]);
        let idx = match owner {
            PlayerId::A => 0,
            PlayerId::B => 1,
        };
        entry[idx] += 1;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::BumpEventFire {
                event,
                player: owner,
            });
        }
    }

    /// Set `tapped` on a card, journaling the prior value if recording.
    pub fn set_tapped(&mut self, iid: &InstanceId, tapped: bool) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        let was = inst.tapped;
        inst.tapped = tapped;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetTapped {
                iid: iid.clone(),
                was,
            });
        }
    }

    /// Set `damage` on a card, journaling the prior value.
    pub fn set_damage(&mut self, iid: &InstanceId, damage: i32) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        let was = inst.damage;
        inst.damage = damage;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetDamage {
                iid: iid.clone(),
                was,
            });
        }
    }

    /// Set `face_down` on a card, journaling the prior value.
    pub fn set_face_down(&mut self, iid: &InstanceId, face_down: bool) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        let was = inst.face_down;
        inst.face_down = face_down;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetFaceDown {
                iid: iid.clone(),
                was,
            });
        }
    }

    /// Set `summoning_sick` on a card, journaling the prior value.
    pub fn set_summoning_sick(&mut self, iid: &InstanceId, summoning_sick: bool) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        let was = inst.summoning_sick;
        inst.summoning_sick = summoning_sick;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetSummoningSick {
                iid: iid.clone(),
                was,
            });
        }
    }

    /// Set the winner, journaling the prior value.
    pub fn set_winner(&mut self, winner: Option<PlayerId>) {
        let was = self.winner;
        self.winner = winner;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetWinner { was });
        }
    }

    pub fn set_phase(&mut self, phase: Phase) {
        let was = self.phase;
        self.phase = phase;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetPhase { was });
        }
    }

    pub fn set_turn(&mut self, turn: u32) {
        let was = self.turn;
        self.turn = turn;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetTurn { was });
        }
    }

    pub fn set_active_player(&mut self, who: PlayerId) {
        let was = self.active_player;
        self.active_player = who;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetActivePlayer { was });
        }
    }

    pub fn set_combat(&mut self, combat: Option<CombatState>) {
        let was = self.combat.clone();
        self.combat = combat;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetCombatState { was });
        }
    }

    /// Replace a card's status_effects vec wholesale, journaling the prior value.
    pub fn set_status_effects(&mut self, iid: &InstanceId, effects: Vec<StatusEffect>) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        let was = std::mem::replace(&mut inst.status_effects, effects);
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetStatusEffects {
                iid: iid.clone(),
                was,
            });
        }
    }

    /// Remove an iid from a player's zone without placing it elsewhere.
    /// Returns the position it was at (for callers that want to follow up with
    /// e.g. an `add_attached`). Returns None if the iid wasn't in that zone.
    pub fn remove_from_zone(
        &mut self,
        iid: &InstanceId,
        owner: PlayerId,
        zone: Zone,
    ) -> Option<usize> {
        let p = self.player_mut(owner);
        let zone_vec = match zone {
            Zone::Board => &mut p.board,
            Zone::Hand => &mut p.hand,
            Zone::Deck => &mut p.deck,
            Zone::Graveyard => &mut p.graveyard,
            Zone::Exile => &mut p.exile,
        };
        let pos = zone_vec.iter().position(|x| x == iid)?;
        zone_vec.remove(pos);
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::RemoveFromZone {
                iid: iid.clone(),
                owner,
                zone,
                was_pos: pos,
            });
        }
        Some(pos)
    }

    /// Append an iid to host's attached vec, journaling the addition.
    pub fn add_attached(&mut self, host: &InstanceId, attached: &InstanceId) {
        let Some(inst) = self.card_pool.get_mut(host) else {
            return;
        };
        inst.attached.push(attached.clone());
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::AddAttached {
                host: host.clone(),
                attached: attached.clone(),
            });
        }
    }

    /// Append an iid to a player's zone, journaling the push. (Counterpart of
    /// `remove_from_zone` — together they let callers detach a card from
    /// attached-limbo and place it back into a zone.)
    pub fn add_to_zone(&mut self, iid: &InstanceId, owner: PlayerId, zone: Zone) {
        let p = self.player_mut(owner);
        let zone_vec = match zone {
            Zone::Board => &mut p.board,
            Zone::Hand => &mut p.hand,
            Zone::Deck => &mut p.deck,
            Zone::Graveyard => &mut p.graveyard,
            Zone::Exile => &mut p.exile,
        };
        zone_vec.push(iid.clone());
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::AddToZone {
                iid: iid.clone(),
                owner,
                zone,
            });
        }
    }

    /// Remove an iid from `host`'s attached vec, journaling the removal at
    /// its position. Returns true if the iid was actually attached to host.
    pub fn remove_attached(&mut self, host: &InstanceId, attached: &InstanceId) -> bool {
        let Some(inst) = self.card_pool.get_mut(host) else {
            return false;
        };
        let Some(pos) = inst.attached.iter().position(|x| x == attached) else {
            return false;
        };
        inst.attached.remove(pos);
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::RemoveAttached {
                host: host.clone(),
                attached: attached.clone(),
                at_pos: pos,
            });
        }
        true
    }

    /// Engine helper: credit a `game.*` action invocation to the affected player.
    pub fn bump_action(&mut self, action: &'static str, who: PlayerId) {
        let entry = self.action_counts.entry(action).or_insert([0, 0]);
        let idx = match who {
            PlayerId::A => 0,
            PlayerId::B => 1,
        };
        entry[idx] += 1;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::BumpAction {
                action,
                player: who,
            });
        }
    }

    /// Convenience: total fires across all events for a given player.
    pub fn total_fires(&self, who: PlayerId) -> u32 {
        let idx = match who {
            PlayerId::A => 0,
            PlayerId::B => 1,
        };
        self.event_fires.values().map(|v| v[idx]).sum()
    }

    fn init_player(
        pid: PlayerId,
        cards: Vec<Card>,
        pool: &mut BTreeMap<InstanceId, CardInstance>,
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
                summoning_sick: false,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardType;
    use crate::game::test_helpers::*;

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
    fn check_loss_detects_empty_deck() {
        let s = GameState::new(deck_of(5, "a"), deck_of(50, "b"));
        assert_eq!(s.check_loss(), Some(PlayerId::A));
        let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        assert_eq!(s.check_loss(), None);
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
        s.card_pool.get_mut(&iid).unwrap().card = card_no_stats("instant", CardType::Instant);
        assert_eq!(s.effective_stats(&iid), (0, 0));
    }

    #[test]
    fn player_id_opponent_swaps() {
        assert_eq!(PlayerId::A.opponent(), PlayerId::B);
        assert_eq!(PlayerId::B.opponent(), PlayerId::A);
    }
}
