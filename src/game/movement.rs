//! Zone movement.
//!
//! The single canonical place where a card's position between zones changes.

use super::journal::JournalEntry;
use super::state::{GameState, InstanceId, PlayerId, PlayerState, Zone};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    NotInZone,
}

fn zone_mut(p: &mut PlayerState, zone: Zone) -> &mut Vec<InstanceId> {
    match zone {
        Zone::Board => &mut p.board,
        Zone::Hand => &mut p.hand,
        Zone::Deck => &mut p.deck,
        Zone::Graveyard => &mut p.graveyard,
        Zone::Exile => &mut p.exile,
    }
}

impl GameState {
    /// Move a card between two zones owned by a single player.
    /// Returns Err if the instance isn't found in the source zone.
    ///
    /// Journaled: records the from-zone position so rollback restores the
    /// card at its original index.
    pub fn move_card(
        &mut self,
        iid: &InstanceId,
        side: PlayerId,
        from: Zone,
        to: Zone,
    ) -> Result<(), MoveError> {
        let p = self.player_mut(side);
        let from_pos = zone_mut(p, from)
            .iter()
            .position(|x| x == iid)
            .ok_or(MoveError::NotInZone)?;
        zone_mut(p, from).remove(from_pos);
        zone_mut(p, to).push(iid.clone());

        if let Some(j) = self.active_journal() {
            j.push(JournalEntry::MoveCard {
                iid: iid.clone(),
                owner: side,
                from_zone: from,
                from_pos,
                to_zone: to,
            });
        }
        Ok(())
    }

    /// RULES P.8: move any cards still attached to `host` into their
    /// own owner's EXILE. Called AFTER `OnDie` (or any equivalent
    /// last-words handler) has run so handlers like trustworthy-lender
    /// can still read `self.attached` and intercept individual cards
    /// (e.g. `game.move(aid, "hand")`) — anything they didn't move
    /// gets exiled here. Idempotent: if the attached list is already
    /// empty, no-op.
    ///
    /// Per-attached owner lookup: a stolen attachment still returns to
    /// its real owner's exile, not the host's owner's exile.
    pub fn exile_remaining_attached(&mut self, host: &InstanceId) {
        let attached_snapshot: Vec<InstanceId> = self
            .card_pool
            .get(host)
            .map(|i| i.attached.clone())
            .unwrap_or_default();
        for aid in &attached_snapshot {
            self.remove_attached(host, aid);
            self.set_face_down(aid, false);
            let owner = self
                .card_pool
                .get(aid)
                .map(|i| i.owner)
                .unwrap_or_else(|| self.active_player);
            self.add_to_zone(aid, owner, Zone::Exile);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::test_helpers::*;

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
    fn journaled_mutations_round_trip_to_original_state() {
        // Open a journal, apply several mutations across move_card and
        // direct field setters, rollback, assert pre-mutation state.
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        let snapshot = format!("{:?}", s);

        s.journal = Some(crate::game::Journal::new());
        s.move_card(&iid, PlayerId::A, Zone::Hand, Zone::Graveyard)
            .unwrap();
        s.set_tapped(&iid, true);
        s.set_damage(&iid, 3.0);
        s.set_face_down(&iid, true);
        s.set_summoning_sick(&iid, true);
        s.set_winner(Some(PlayerId::A));
        s.bump_action("test", PlayerId::A);
        s.bump_event_fire(crate::card::EventName::OnDie, PlayerId::B);

        let after_mutations = format!("{:?}", s);
        assert_ne!(snapshot, after_mutations, "mutations should have visible effect");

        let journal = s.journal.take().unwrap();
        journal.rollback(&mut s);
        assert!(s.journal.is_none());

        let restored = format!("{:?}", s);
        assert_eq!(snapshot, restored, "rollback should restore exact prior state");
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
}
