//! Journal & rollback — see JOURNAL.md for the multi-session plan.
//!
//! Records every mutation through `GameState`'s journaled helpers. Each entry
//! carries enough information to undo itself. `Journal::rollback` applies
//! inverses in reverse order, leaving the state byte-identical to pre-mutation.
//!
//! Session 1 scope: the data types, the apply-inverse routine, helpers on
//! `GameState` for the core mutations, and `movement.rs::move_card` rewritten
//! through them. Other subsystems still mutate state directly — Sessions 2–3
//! convert them.

use super::state::{GameState, InstanceId, PlayerId, Zone};
use crate::card::EventName;

/// One mutation entry, carrying the data needed to undo it.
#[derive(Debug, Clone)]
pub enum JournalEntry {
    SetTapped {
        iid: InstanceId,
        was: bool,
    },
    SetDamage {
        iid: InstanceId,
        was: i32,
    },
    SetFaceDown {
        iid: InstanceId,
        was: bool,
    },
    SetSummoningSick {
        iid: InstanceId,
        was: bool,
    },
    /// Moved `iid` from `from_zone[from_pos]` to the end of `to_zone`.
    /// Inverse: pop from end of `to_zone`, insert at `from_pos` of `from_zone`.
    MoveCard {
        iid: InstanceId,
        owner: PlayerId,
        from_zone: Zone,
        from_pos: usize,
        to_zone: Zone,
    },
    /// Incremented `action_counts[action][player]` by 1.
    /// Inverse: decrement; if both counts go to 0, remove the key entirely
    /// so round-trip equality holds against pre-mutation state.
    BumpAction {
        action: &'static str,
        player: PlayerId,
    },
    BumpEventFire {
        event: EventName,
        player: PlayerId,
    },
    SetWinner {
        was: Option<PlayerId>,
    },
}

/// Recording journal — owns a sequence of entries from a single "session"
/// of mutations. Rollback consumes the journal.
#[derive(Debug, Clone, Default)]
pub struct Journal {
    entries: Vec<JournalEntry>,
}

impl Journal {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, entry: JournalEntry) {
        self.entries.push(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Apply inverses of every entry, in reverse order. Consumes the journal.
    pub fn rollback(self, state: &mut GameState) {
        for entry in self.entries.into_iter().rev() {
            apply_inverse(state, entry);
        }
    }
}

fn zone_mut(p: &mut super::state::PlayerState, zone: Zone) -> &mut Vec<InstanceId> {
    match zone {
        Zone::Board => &mut p.board,
        Zone::Hand => &mut p.hand,
        Zone::Deck => &mut p.deck,
        Zone::Graveyard => &mut p.graveyard,
        Zone::Exile => &mut p.exile,
    }
}

fn apply_inverse(state: &mut GameState, entry: JournalEntry) {
    match entry {
        JournalEntry::SetTapped { iid, was } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.tapped = was;
            }
        }
        JournalEntry::SetDamage { iid, was } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.damage = was;
            }
        }
        JournalEntry::SetFaceDown { iid, was } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.face_down = was;
            }
        }
        JournalEntry::SetSummoningSick { iid, was } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.summoning_sick = was;
            }
        }
        JournalEntry::MoveCard {
            iid,
            owner,
            from_zone,
            from_pos,
            to_zone,
        } => {
            let p = state.player_mut(owner);
            let dst = zone_mut(p, to_zone);
            // Pop the iid we appended. It must be at the end.
            if let Some(last) = dst.last() {
                debug_assert_eq!(*last, iid, "move-card inverse: iid mismatch at to_zone tail");
                dst.pop();
            }
            let src = zone_mut(p, from_zone);
            src.insert(from_pos, iid);
        }
        JournalEntry::BumpAction { action, player } => {
            let remove_key = if let Some(entry) = state.action_counts.get_mut(action) {
                let idx = match player {
                    PlayerId::A => 0,
                    PlayerId::B => 1,
                };
                if entry[idx] > 0 {
                    entry[idx] -= 1;
                }
                entry[0] == 0 && entry[1] == 0
            } else {
                false
            };
            if remove_key {
                state.action_counts.remove(action);
            }
        }
        JournalEntry::BumpEventFire { event, player } => {
            let remove_key = if let Some(entry) = state.event_fires.get_mut(&event) {
                let idx = match player {
                    PlayerId::A => 0,
                    PlayerId::B => 1,
                };
                if entry[idx] > 0 {
                    entry[idx] -= 1;
                }
                entry[0] == 0 && entry[1] == 0
            } else {
                false
            };
            if remove_key {
                state.event_fires.remove(&event);
            }
        }
        JournalEntry::SetWinner { was } => {
            state.winner = was;
        }
    }
}
