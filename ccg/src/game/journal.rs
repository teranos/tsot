//! Journal & rollback — see JOURNAL.md for the multi-session plan.
//!
//! Records every mutation through `GameState`'s journaled helpers. Each entry
//! carries enough information to apply both forward (replay) and reverse
//! (rollback) the mutation.

use super::state::{
    CombatState, GameState, InstanceId, Modifier, Phase, PlayerId, Sleeve, StatusEffect, Zone,
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
        was: f32,
        now: f32,
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
    SetAttackedThisTurn {
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
    SetController {
        iid: InstanceId,
        was: PlayerId,
        now: PlayerId,
    },
    SetCombatState {
        was: Option<CombatState>,
        now: Option<CombatState>,
    },
    SetCreatureAttackedThisTurn {
        was: bool,
        now: bool,
    },
    /// P.35: per-player Symbol-cast cap flag. `player_idx` is 0 (A) or
    /// 1 (B) — matching `GameState::symbol_cast_this_turn`'s indexing.
    SetSymbolCastThisTurn {
        player_idx: usize,
        was: bool,
        now: bool,
    },
    SetPriorityState {
        was: Option<super::PriorityState>,
        now: Option<super::PriorityState>,
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
    /// Z.8: minted a fresh cardless sleeve into the pool — a mutation cast
    /// (P.26) vacating its own sleeve. Forward: insert. Inverse: remove.
    MintCardlessSleeve {
        iid: InstanceId,
        owner: PlayerId,
    },
    /// Z.7: fused a card into a host's `same_sleeve` list. Inverse: pop last.
    AddSameSleeve {
        host: InstanceId,
        sleeved: InstanceId,
    },
    /// Z.7: removed a fused card from a host's `same_sleeve` list.
    /// Inverse: re-insert at `at_pos`.
    RemoveSameSleeve {
        host: InstanceId,
        sleeved: InstanceId,
        at_pos: usize,
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
    /// Inserted an iid at position 0 of a zone (deck-top placement for
    /// cantrips like Sprout). Inverse: remove from position 0.
    AddToZoneTop {
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
    /// Cleared all `EotStatBoost` modifiers from a single card's `modifiers`
    /// vec at end-of-turn. `removed` captures the variants in original order
    /// so a rollback can splice them back in. Forward: re-clear.
    ClearEotModifiers {
        iid: InstanceId,
        /// Original positions and values, sorted by index ascending.
        /// Rollback reinserts at the stored positions.
        removed: Vec<(usize, Modifier)>,
    },
    // --- Scheduled-state registries ---
    // These `Vec` fields persist ACROSS turns, so — like every other
    // field — a mutation inside a preview/rollout MUST be journaled or
    // rollback leaves the real state corrupted. Whole-field was/now, same
    // shape as `SetCombatState`. See the JOURNALING CONTRACT on GameState.
    SetDelayedTriggers {
        was: Vec<super::DelayedTrigger>,
        now: Vec<super::DelayedTrigger>,
    },
    SetPendingMainPhaseReturns {
        was: Vec<InstanceId>,
        now: Vec<InstanceId>,
    },
    SetExtraTurnsPending {
        was: Vec<PlayerId>,
        now: Vec<PlayerId>,
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
        // O3: fan out every mutation to the trace bus. Cheap no-op
        // when trace is disabled (native EA / probe paths); cloning
        // the entry once per push is the visibility cost we pay
        // when trace is on. Push happens BEFORE the local append so
        // the buffer order matches execution order.
        if crate::trace::is_enabled() {
            crate::trace::push(crate::trace::TraceEvent::Mutation {
                at_us: crate::trace::now_us(),
                entry: entry.clone(),
            });
        }
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
        JournalEntry::SetAttackedThisTurn { iid, was, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.attacked_this_turn = was;
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
        JournalEntry::SetController { iid, was, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.controller = was;
            }
        }
        JournalEntry::SetCombatState { was, .. } => {
            state.combat = was;
        }
        JournalEntry::SetDelayedTriggers { was, .. } => {
            state.delayed_triggers = was;
        }
        JournalEntry::SetPendingMainPhaseReturns { was, .. } => {
            state.pending_main_phase_returns = was;
        }
        JournalEntry::SetExtraTurnsPending { was, .. } => {
            state.extra_turns_pending = was;
        }
        JournalEntry::SetCreatureAttackedThisTurn { was, .. } => {
            state.creature_attacked_this_turn = was;
        }
        JournalEntry::SetSymbolCastThisTurn { player_idx, was, .. } => {
            state.symbol_cast_this_turn[player_idx] = was;
        }
        JournalEntry::SetPriorityState { was, .. } => {
            state.priority = was;
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
        JournalEntry::MintCardlessSleeve { iid, .. } => {
            state.card_pool.remove(&iid);
        }
        JournalEntry::AddSameSleeve { host, sleeved } => {
            if let Some(inst) = state.card_pool.get_mut(&host) {
                if let Some(last) = inst.same_sleeve.last() {
                    debug_assert_eq!(
                        *last, sleeved,
                        "add-same-sleeve inverse: iid mismatch at tail"
                    );
                    inst.same_sleeve.pop();
                }
            }
        }
        JournalEntry::RemoveSameSleeve {
            host,
            sleeved,
            at_pos,
        } => {
            if let Some(inst) = state.card_pool.get_mut(&host) {
                inst.same_sleeve.insert(at_pos, sleeved);
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
        JournalEntry::AddToZoneTop { iid, owner, zone } => {
            let p = state.player_mut(owner);
            let v = zone_mut(p, zone);
            if let Some(first) = v.first() {
                debug_assert_eq!(*first, iid, "add-to-zone-top inverse: iid mismatch at head");
                v.remove(0);
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
        JournalEntry::ClearEotModifiers { iid, removed } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                // Reinsert in ascending-index order. After each insertion
                // the tail indices shift, but we recorded positions BEFORE
                // any removal so iterating ascending is correct.
                for (pos, m) in removed {
                    if pos <= inst.modifiers.len() {
                        inst.modifiers.insert(pos, m);
                    } else {
                        inst.modifiers.push(m);
                    }
                }
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
        JournalEntry::SetAttackedThisTurn { iid, now, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.attacked_this_turn = now;
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
        JournalEntry::SetController { iid, now, .. } => {
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.controller = now;
            }
        }
        JournalEntry::SetCombatState { now, .. } => {
            state.combat = now;
        }
        JournalEntry::SetDelayedTriggers { now, .. } => {
            state.delayed_triggers = now;
        }
        JournalEntry::SetPendingMainPhaseReturns { now, .. } => {
            state.pending_main_phase_returns = now;
        }
        JournalEntry::SetExtraTurnsPending { now, .. } => {
            state.extra_turns_pending = now;
        }
        JournalEntry::SetCreatureAttackedThisTurn { now, .. } => {
            state.creature_attacked_this_turn = now;
        }
        JournalEntry::SetSymbolCastThisTurn { player_idx, now, .. } => {
            state.symbol_cast_this_turn[player_idx] = now;
        }
        JournalEntry::SetPriorityState { now, .. } => {
            state.priority = now;
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
        JournalEntry::MintCardlessSleeve { iid, owner } => {
            state
                .card_pool
                .insert(iid.clone(), Sleeve::cardless(iid, owner));
        }
        JournalEntry::AddSameSleeve { host, sleeved } => {
            if let Some(inst) = state.card_pool.get_mut(&host) {
                inst.same_sleeve.push(sleeved);
            }
        }
        JournalEntry::RemoveSameSleeve {
            host,
            sleeved: _,
            at_pos,
        } => {
            if let Some(inst) = state.card_pool.get_mut(&host) {
                inst.same_sleeve.remove(at_pos);
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
        JournalEntry::AddToZoneTop { iid, owner, zone } => {
            let p = state.player_mut(owner);
            zone_mut(p, zone).insert(0, iid);
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
        JournalEntry::ClearEotModifiers { iid, removed: _ } => {
            // Forward: re-strip all EOT modifiers. (The `removed` field is
            // used only for rollback; replay just re-applies the operation.)
            if let Some(inst) = state.card_pool.get_mut(&iid) {
                inst.modifiers
                    .retain(|m| !matches!(m, Modifier::EotStatBoost { .. }));
            }
        }
    }
}
