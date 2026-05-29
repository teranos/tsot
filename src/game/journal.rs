//! Journal & rollback — see JOURNAL.md for the multi-session plan.
//!
//! Records every mutation through `GameState`'s journaled helpers. Each entry
//! carries enough information to apply both forward (replay) and reverse
//! (rollback) the mutation.

use super::state::{
    CombatState, GameState, InstanceId, Modifier, Phase, PlayerId, StatusEffect, Zone,
};
use crate::card::EventName;
use serde::{Deserialize, Serialize};

/// One mutation entry. `Set*` variants carry both `was` and `now` so the
/// entry can be applied forward or reverse. Bump-style entries are
/// self-symmetric (+1 forward, -1 reverse). Move/Add/Remove variants store
/// positional data sufficient for both directions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JournalEntry {
    SetTapped {
        iid: InstanceId,
        was: bool,
        now: bool,
    },
    SetDamage {
        iid: InstanceId,
        was: i32,
        now: i32,
    },
    SetFaceDown {
        iid: InstanceId,
        was: bool,
        now: bool,
    },
    SetSummoningSick {
        iid: InstanceId,
        was: bool,
        now: bool,
    },
    MoveCard {
        iid: InstanceId,
        owner: PlayerId,
        from_zone: Zone,
        from_pos: usize,
        to_zone: Zone,
    },
    BumpAction {
        action: String,
        player: PlayerId,
    },
    BumpEventFire {
        event: EventName,
        player: PlayerId,
    },
    SetWinner {
        was: Option<PlayerId>,
        now: Option<PlayerId>,
    },
    SetPhase {
        was: Phase,
        now: Phase,
    },
    SetTurn {
        was: u32,
        now: u32,
    },
    SetActivePlayer {
        was: PlayerId,
        now: PlayerId,
    },
    SetCombatState {
        was: Option<CombatState>,
        now: Option<CombatState>,
    },
    SetStatusEffects {
        iid: InstanceId,
        was: Vec<StatusEffect>,
        now: Vec<StatusEffect>,
    },
    AddAttached {
        host: InstanceId,
        attached: InstanceId,
    },
    RemoveFromZone {
        iid: InstanceId,
        owner: PlayerId,
        zone: Zone,
        was_pos: usize,
    },
    AddToZone {
        iid: InstanceId,
        owner: PlayerId,
        zone: Zone,
    },
    RemoveAttached {
        host: InstanceId,
        attached: InstanceId,
        at_pos: usize,
    },
    /// Appended a `Modifier` to a card's `modifiers` vec. Inverse: pop last.
    /// Forward: push to end.
    AddModifier {
        iid: InstanceId,
        modifier: Modifier,
    },
}

/// Journal — sequence of mutation entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// Move every entry from `other` to the end of `self`. Used to commit
    /// a preview's mutations into the long-lived replay journal once the
    /// preview is accepted.
    pub fn extend_from(&mut self, other: &mut Journal) {
        self.entries.append(&mut other.entries);
    }

    /// Apply inverses of every entry, in reverse order. Consumes the journal.
    pub fn rollback(self, state: &mut GameState) {
        for entry in self.entries.into_iter().rev() {
            apply_inverse(state, entry);
        }
    }

    /// Apply every entry forward, in order. Used to replay a recorded game
    /// starting from a freshly-built initial state. Consumes the journal.
    pub fn replay_forward(self, state: &mut GameState) {
        for entry in self.entries {
            apply_forward(state, entry);
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

fn bump_action_count(state: &mut GameState, action: &str, player: PlayerId, delta: i32) {
    let entry = state
        .action_counts
        .entry(action.to_string())
        .or_insert([0, 0]);
    let idx = match player {
        PlayerId::A => 0,
        PlayerId::B => 1,
    };
    if delta > 0 {
        entry[idx] += delta as u32;
    } else if entry[idx] > 0 {
        entry[idx] = entry[idx].saturating_sub((-delta) as u32);
    }
    if entry[0] == 0 && entry[1] == 0 {
        state.action_counts.remove(action);
    }
}

fn bump_event_fire_count(
    state: &mut GameState,
    event: EventName,
    player: PlayerId,
    delta: i32,
) {
    let entry = state.event_fires.entry(event).or_insert([0, 0]);
    let idx = match player {
        PlayerId::A => 0,
        PlayerId::B => 1,
    };
    if delta > 0 {
        entry[idx] += delta as u32;
    } else if entry[idx] > 0 {
        entry[idx] = entry[idx].saturating_sub((-delta) as u32);
    }
    if entry[0] == 0 && entry[1] == 0 {
        state.event_fires.remove(&event);
    }
}

fn apply_inverse(state: &mut GameState, entry: JournalEntry) {
    match entry {
        JournalEntry::SetTapped { iid, was, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.tapped = was;
            }
        }
        JournalEntry::SetDamage { iid, was, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.damage = was;
            }
        }
        JournalEntry::SetFaceDown { iid, was, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.face_down = was;
            }
        }
        JournalEntry::SetSummoningSick { iid, was, .. } => {
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
            if let Some(last) = dst.last() {
                debug_assert_eq!(*last, iid, "move-card inverse: iid mismatch at to_zone tail");
                dst.pop();
            }
            let src = zone_mut(p, from_zone);
            src.insert(from_pos, iid);
        }
        JournalEntry::BumpAction { action, player } => {
            bump_action_count(state, &action, player, -1);
        }
        JournalEntry::BumpEventFire { event, player } => {
            bump_event_fire_count(state, event, player, -1);
        }
        JournalEntry::SetWinner { was, .. } => {
            state.winner = was;
        }
        JournalEntry::SetPhase { was, .. } => {
            state.phase = was;
        }
        JournalEntry::SetTurn { was, .. } => {
            state.turn = was;
        }
        JournalEntry::SetActivePlayer { was, .. } => {
            state.active_player = was;
        }
        JournalEntry::SetCombatState { was, .. } => {
            state.combat = was;
        }
        JournalEntry::SetStatusEffects { iid, was, .. } => {
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
        JournalEntry::AddModifier { iid, modifier: _ } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.modifiers.pop();
            }
        }
    }
}

fn apply_forward(state: &mut GameState, entry: JournalEntry) {
    match entry {
        JournalEntry::SetTapped { iid, now, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.tapped = now;
            }
        }
        JournalEntry::SetDamage { iid, now, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.damage = now;
            }
        }
        JournalEntry::SetFaceDown { iid, now, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.face_down = now;
            }
        }
        JournalEntry::SetSummoningSick { iid, now, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.summoning_sick = now;
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
            // Forward: remove at from_pos in from_zone, push to to_zone end.
            zone_mut(p, from_zone).remove(from_pos);
            zone_mut(p, to_zone).push(iid);
        }
        JournalEntry::BumpAction { action, player } => {
            bump_action_count(state, &action, player, 1);
        }
        JournalEntry::BumpEventFire { event, player } => {
            bump_event_fire_count(state, event, player, 1);
        }
        JournalEntry::SetWinner { now, .. } => {
            state.winner = now;
        }
        JournalEntry::SetPhase { now, .. } => {
            state.phase = now;
        }
        JournalEntry::SetTurn { now, .. } => {
            state.turn = now;
        }
        JournalEntry::SetActivePlayer { now, .. } => {
            state.active_player = now;
        }
        JournalEntry::SetCombatState { now, .. } => {
            state.combat = now;
        }
        JournalEntry::SetStatusEffects { iid, now, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.status_effects = now;
            }
        }
        JournalEntry::AddAttached { host, attached } => {
            if let Some(inst) = state.card_pool.get_mut(&host) {
                inst.attached.push(attached);
            }
        }
        JournalEntry::RemoveFromZone {
            iid,
            owner,
            zone,
            was_pos,
        } => {
            let p = state.player_mut(owner);
            let v = zone_mut(p, zone);
            // Forward: card should be at was_pos; remove it.
            debug_assert_eq!(v.get(was_pos), Some(&iid));
            v.remove(was_pos);
        }
        JournalEntry::AddToZone { iid, owner, zone } => {
            let p = state.player_mut(owner);
            zone_mut(p, zone).push(iid);
        }
        JournalEntry::RemoveAttached {
            host,
            attached: _,
            at_pos,
        } => {
            if let Some(inst) = state.card_pool.get_mut(&host) {
                inst.attached.remove(at_pos);
            }
        }
        JournalEntry::AddModifier { iid, modifier } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.modifiers.push(modifier);
            }
        }
    }
}
