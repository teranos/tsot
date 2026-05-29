//! Zone movement.
//!
//! The single canonical place where a card's position between zones changes.

use super::state::{GameState, InstanceId, PlayerId, Zone};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    NotInZone,
}

impl GameState {
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
    fn move_card_errors_when_not_in_source() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        assert_eq!(
            s.move_card(&iid, PlayerId::A, Zone::Graveyard, Zone::Exile),
            Err(MoveError::NotInZone)
        );
    }
}
