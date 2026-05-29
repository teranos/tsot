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

use super::state::{
    CombatState, GameState, InstanceId, Phase, PlayerId, StatusEffect, Zone,
};
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
    SetPhase {
        was: Phase,
    },
    SetTurn {
        was: u32,
    },
    SetActivePlayer {
        was: PlayerId,
    },
    /// Coarse: replaces the entire combat state. Sufficient for declare /
    /// confirm transitions; finer-grained "add blocker to attack" entries
    /// could be added if needed.
    SetCombatState {
        was: Option<CombatState>,
    },
    /// Coarse: replaces the full status_effects vec on a card. Inserts and
    /// removes are rare enough that this is simpler than fine-grained.
    SetStatusEffects {
        iid: InstanceId,
        was: Vec<StatusEffect>,
    },
    /// Appended an iid to `host.attached`. Inverse: pop last from
    /// `host.attached` (must match `attached`).
    AddAttached {
        host: InstanceId,
        attached: InstanceId,
    },
    /// Removed an iid from a player's zone (without placing it elsewhere).
    /// Inverse: insert iid back at `was_pos` in the zone.
    RemoveFromZone {
        iid: InstanceId,
        owner: PlayerId,
        zone: Zone,
        was_pos: usize,
    },
    /// Pushed an iid to the end of a player's zone (without removing it from
    /// elsewhere). Inverse: pop from end of zone (must match `iid`).
    AddToZone {
        iid: InstanceId,
        owner: PlayerId,
        zone: Zone,
    },
    /// Removed an iid from host's attached vec at `at_pos`. Inverse: insert
    /// back at `at_pos`.
    RemoveAttached {
        host: InstanceId,
        attached: InstanceId,
        at_pos: usize,
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
        JournalEntry::SetPhase { was } => {
            state.phase = was;
        }
        JournalEntry::SetTurn { was } => {
            state.turn = was;
        }
        JournalEntry::SetActivePlayer { was } => {
            state.active_player = was;
        }
        JournalEntry::SetCombatState { was } => {
            state.combat = was;
        }
        JournalEntry::SetStatusEffects { iid, was } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.status_effects = was;
            }
        }
        JournalEntry::AddAttached { host, attached } => {
            if let Some(inst) = state.card_pool.get_mut(&host) {
                if let Some(last) = inst.attached.last() {
                    debug_assert_eq!(
                        *last, attached,
                        "add-attached inverse: iid mismatch at tail"
                    );
                    inst.attached.pop();
                }
            }
        }
        JournalEntry::RemoveFromZone {
            iid,
            owner,
            zone,
            was_pos,
        } => {
            let p = state.player_mut(owner);
            zone_mut(p, zone).insert(was_pos, iid);
        }
        JournalEntry::AddToZone { iid, owner, zone } => {
            let p = state.player_mut(owner);
            let v = zone_mut(p, zone);
            if let Some(last) = v.last() {
                debug_assert_eq!(*last, iid, "add-to-zone inverse: iid mismatch at tail");
                v.pop();
            }
        }
        JournalEntry::RemoveAttached {
            host,
            attached,
            at_pos,
        } => {
            if let Some(inst) = state.card_pool.get_mut(&host) {
                inst.attached.insert(at_pos, attached);
            }
        }
    }
}
