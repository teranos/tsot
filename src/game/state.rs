//! Core game-state types and basic accessors.
//!
//! Mirrors RULES.md sections F, U, L, S, Z, T, C.

use crate::card::{Card, EventName};
use std::collections::BTreeMap;

/// F.2: exactly two players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    Untap,
    Draw,
    Main1,
    Combat,
    Main2,
    End,
}

/// Z.1–Z.5: per-player zones. Z.6 (ATTACHED) is encoded as a child list under each on-board instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Zone {
    Board,
    Deck,
    Hand,
    Graveyard,
    Exile,
}

pub type InstanceId = String;

/// A specific copy of a card in the game.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Modifier {
    /// e.g., +1/+1
    StatBoost { x: i32, y: i32 },
    /// e.g., Companion Bird grants flying while attached
    GainsFlying,
    /// e.g., Flesh-eating Plant
    CantAttack,
}

/// Status effects with bounded duration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum StatusEffect {
    /// Stinging-bee: skip the next N untap steps
    SkipUntap(u32),
}

/// Per-player state. Zones reference instances by ID; the canonical
/// CardInstance lives in GameState.card_pool.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PlayerState {
    pub board: Vec<InstanceId>,
    pub hand: Vec<InstanceId>,
    pub deck: Vec<InstanceId>, // first element = top of deck (V.1)
    pub graveyard: Vec<InstanceId>,
    pub exile: Vec<InstanceId>,
}

/// In-progress combat state during the Combat phase.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CombatState {
    AwaitingAttackers,
    AwaitingBlockers { attacks: Vec<AttackDecl> },
}

/// One attacker and zero-or-more blockers assigned to it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AttackDecl {
    pub attacker: InstanceId,
    pub blockers: Vec<InstanceId>,
}

/// An item on the response chain. STACK Phase 1 only models played cards
/// (creatures and instants); triggered abilities resolve inline per the
/// inline-triggers design.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StackItem {
    PlayedCard {
        card: InstanceId,
        controller: PlayerId,
        /// Resolution-time data. The cast's non-hand cost is paid at
        /// announce time, but HAND payments + destination move + handler
        /// fires happen at resolution. The choices have to ride along.
        choices: super::PlayChoices,
    },
}

/// Response window state. `None` on `GameState.priority` means no window is
/// currently open; engine has direct control. `Some` means a window is open
/// and players may submit responses or pass.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PriorityState {
    /// Bottom-to-top: `chain[0]` resolves last; `chain[N-1]` is the top
    /// (next to resolve once both players pass).
    pub chain: Vec<StackItem>,
    /// Who has priority right now.
    pub next_to_act: PlayerId,
    /// 0, 1, or 2. Two consecutive passes → top of chain resolves
    /// (or window closes if chain is empty).
    pub consecutive_passes: u8,
}

/// Errors from the priority/pass engine. Phase 1 only flags structural
/// mistakes (no window when one is required, or double-open).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityError {
    /// `pass_priority` or `respond_with` called with `state.priority == None`.
    NoWindowOpen,
    /// `open_response_window` called while a window is already open. Phase 1
    /// doesn't nest windows; Phase 2 may relax this.
    WindowAlreadyOpen,
}

/// The full game state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    pub action_counts: BTreeMap<String, [u32; 2]>,
    /// Optional per-action mutation journal. `None` = no recording. Used for
    /// preview-and-rollback (sim's "would this play kill me?" check). When
    /// `Some`, every mutation pushes here instead of `replay_journal`.
    pub journal: Option<super::Journal>,
    /// Optional game-long mutation journal. Opened at game start by callers
    /// who want a complete replay. Helpers push here only when `journal` is
    /// `None` (so committed previews are merged in via `extend_from` and
    /// rolled-back ones leave it untouched).
    pub replay_journal: Option<super::Journal>,
    /// Open response window state, or `None` if no window is currently open.
    /// STACK Phase 1 introduces the type; window-openers and resolution loop
    /// arrive in subsequent steps.
    pub priority: Option<PriorityState>,
    /// Set true the first time `declare_attacker` succeeds in the current turn,
    /// reset to false on End → Untap transition. Read by cards whose effect
    /// scales with whether combat happened (e.g., "draw 3 if a creature
    /// attacked this turn, otherwise 2"). Global, not per-player: a 1v1 game
    /// only has one attacking side per turn anyway.
    #[serde(default)]
    pub creature_attacked_this_turn: bool,
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
            priority: None,
            creature_attacked_this_turn: false,
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

    /// Set `tapped` on a card, journaling both the prior and new value
    /// so the entry supports both rollback and forward-replay.
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
                now: tapped,
            });
        }
    }

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
                now: damage,
            });
        }
    }

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
                now: face_down,
            });
        }
    }

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
                now: summoning_sick,
            });
        }
    }

    pub fn set_winner(&mut self, winner: Option<PlayerId>) {
        let was = self.winner;
        self.winner = winner;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetWinner { was, now: winner });
        }
    }

    pub fn set_phase(&mut self, phase: Phase) {
        let was = self.phase;
        self.phase = phase;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetPhase { was, now: phase });
        }
    }

    pub fn set_turn(&mut self, turn: u32) {
        let was = self.turn;
        self.turn = turn;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetTurn { was, now: turn });
        }
    }

    /// Journal-aware setter for a card's controller. Used by theft effects
    /// (e.g., opponent-draw) that take cards from one player into the
    /// other's hand. Owner is immutable per T.2; controller is what changes.
    pub fn set_controller(&mut self, iid: &InstanceId, now: PlayerId) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        let was = inst.controller;
        if was == now {
            return;
        }
        inst.controller = now;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetController {
                iid: iid.clone(),
                was,
                now,
            });
        }
    }

    pub fn set_active_player(&mut self, who: PlayerId) {
        let was = self.active_player;
        self.active_player = who;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetActivePlayer { was, now: who });
        }
    }

    pub fn set_combat(&mut self, combat: Option<CombatState>) {
        let was = self.combat.clone();
        self.combat = combat.clone();
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetCombatState { was, now: combat });
        }
    }

    pub fn set_creature_attacked_this_turn(&mut self, now: bool) {
        let was = self.creature_attacked_this_turn;
        if was == now {
            return;
        }
        self.creature_attacked_this_turn = now;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetCreatureAttackedThisTurn { was, now });
        }
    }

    /// Journal-aware setter for the priority/response-window field. Skips a
    /// no-op write so the journal stays tight.
    pub fn set_priority(&mut self, now: Option<PriorityState>) {
        let was = self.priority.clone();
        if was == now {
            return;
        }
        self.priority = now.clone();
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetPriorityState { was, now });
        }
    }

    /// R.1.a — open a response window with `item` already on the chain. Per
    /// R.7, the active player gets priority first. Errors if a window is
    /// already open (no nesting in Phase 1).
    pub fn open_response_window(&mut self, item: StackItem) -> Result<(), PriorityError> {
        if self.priority.is_some() {
            return Err(PriorityError::WindowAlreadyOpen);
        }
        let active = self.active_player;
        self.set_priority(Some(PriorityState {
            chain: vec![item],
            next_to_act: active,
            consecutive_passes: 0,
        }));
        Ok(())
    }

    /// R.1.b — open a response window with no chain item. The triggering
    /// event (an attack declaration) has already happened by the time this
    /// is called — the window exists so responders can add casts to the
    /// chain before any consequential triggers fire. Closes naturally after
    /// two consecutive passes on an empty chain.
    pub fn open_response_window_empty(&mut self) -> Result<(), PriorityError> {
        if self.priority.is_some() {
            return Err(PriorityError::WindowAlreadyOpen);
        }
        let active = self.active_player;
        self.set_priority(Some(PriorityState {
            chain: Vec::new(),
            next_to_act: active,
            consecutive_passes: 0,
        }));
        Ok(())
    }

    /// Current priority holder passes. Returns `Ok(Some(popped))` when this
    /// is the second consecutive pass — the top of the chain has resolved
    /// and is handed back to the caller for it to apply the resolution.
    /// Returns `Ok(None)` when priority just hands to the other player.
    ///
    /// After a pop: if the chain still has items, priority returns to the
    /// game's active player (R.1: active player gets priority after each
    /// resolution) with the pass counter reset. If the chain is empty, the
    /// window closes (`priority = None`).
    pub fn pass_priority(&mut self) -> Result<Option<StackItem>, PriorityError> {
        let mut p = self.priority.clone().ok_or(PriorityError::NoWindowOpen)?;
        p.consecutive_passes = p.consecutive_passes.saturating_add(1);
        if p.consecutive_passes < 2 {
            p.next_to_act = p.next_to_act.opponent();
            self.set_priority(Some(p));
            return Ok(None);
        }
        // Two passes in a row → resolve top of chain.
        let popped = p.chain.pop();
        if p.chain.is_empty() {
            self.set_priority(None);
        } else {
            p.consecutive_passes = 0;
            p.next_to_act = self.active_player;
            self.set_priority(Some(p));
        }
        Ok(popped)
    }

    /// Push a new item onto the chain in response to whatever's currently on
    /// top. Resets the pass counter and hands priority to the other player.
    pub fn respond_with(&mut self, item: StackItem) -> Result<(), PriorityError> {
        let mut p = self.priority.clone().ok_or(PriorityError::NoWindowOpen)?;
        let responder = p.next_to_act;
        p.chain.push(item);
        p.consecutive_passes = 0;
        p.next_to_act = responder.opponent();
        self.set_priority(Some(p));
        Ok(())
    }

    /// Remove the top of the chain without resolving it (counterspell-style).
    /// Returns the countered item, or None if the chain is empty / no window
    /// is open. After counter, the pass counter resets and priority returns
    /// to the active player (R.7) — the outer driver will close the window
    /// naturally if nothing else gets played.
    pub fn counter_top(&mut self) -> Option<StackItem> {
        let mut p = self.priority.as_ref()?.clone();
        let removed = p.chain.pop()?;
        p.consecutive_passes = 0;
        p.next_to_act = self.active_player;
        self.set_priority(Some(p));
        Some(removed)
    }

    /// Remove the chain item whose card matches `target`. Same semantics as
    /// `counter_top` but selects by InstanceId instead of always popping the
    /// top — lets cards like DTST-creature's "counter target card on the
    /// stack" pick any chain item, not just the one directly underneath.
    /// Returns the removed item or None if no chain item matches.
    pub fn counter_target(&mut self, target: &InstanceId) -> Option<StackItem> {
        let mut p = self.priority.as_ref()?.clone();
        let pos = p.chain.iter().position(|item| {
            let StackItem::PlayedCard { card, .. } = item;
            card == target
        })?;
        let removed = p.chain.remove(pos);
        p.consecutive_passes = 0;
        p.next_to_act = self.active_player;
        self.set_priority(Some(p));
        Some(removed)
    }

    /// Phase 2 introspection (X-E.1): instances in `player`'s hand that
    /// could be cast as a response right now. Filter: instant timing,
    /// HAND / MILL / GRAVEYARD cost (no X, no SACRIFICE/SELF), with the
    /// resources available to pay each component. Same surface the UI will
    /// use to populate the "respond?" prompt.
    pub fn playable_responses(&self, player: PlayerId) -> Vec<InstanceId> {
        let p = self.player(player);
        let hand = &p.hand;
        let gy_len = p.graveyard.len();
        let deck_len = p.deck.len();
        hand.iter()
            .filter(|iid| {
                let Some(inst) = self.card_pool.get(*iid) else {
                    return false;
                };
                if inst.card.kind != crate::card::CardType::Spell {
                    return false;
                }
                if inst.card.timing != Some(crate::card::Timing::Instant) {
                    return false;
                }
                let mut hand_need: usize = 0;
                let mut mill_need: usize = 0;
                let mut gy_need: usize = 0;
                for c in &inst.card.cost {
                    if c.is_x {
                        return false;
                    }
                    let amount = c.amount.max(0) as usize;
                    match c.source {
                        crate::card::CostSource::Hand => hand_need += amount,
                        crate::card::CostSource::Mill => mill_need += amount,
                        crate::card::CostSource::Graveyard => gy_need += amount,
                        _ => return false,
                    }
                }
                hand.len() > hand_need && deck_len >= mill_need && gy_len >= gy_need
            })
            .cloned()
            .collect()
    }

    /// Phase 2 introspection (X-E.2): targets for a counter effect — the
    /// InstanceIds of cards currently on the response chain. Empty if no
    /// window is open. The UI consumes this to populate the target picker
    /// for "counter target spell"; the response policy can use it for
    /// X.2-style "skip if no legal target" auto-pass.
    pub fn legal_counter_targets(&self) -> Vec<InstanceId> {
        self.priority
            .as_ref()
            .map(|p| {
                p.chain
                    .iter()
                    .map(|item| {
                        let StackItem::PlayedCard { card, .. } = item;
                        card.clone()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }


    pub fn set_status_effects(&mut self, iid: &InstanceId, effects: Vec<StatusEffect>) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        let was = std::mem::replace(&mut inst.status_effects, effects.clone());
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetStatusEffects {
                iid: iid.clone(),
                was,
                now: effects,
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

    /// Append a `Modifier` to a card's `modifiers` vec, journaling the addition.
    pub fn add_modifier(&mut self, iid: &InstanceId, modifier: Modifier) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        inst.modifiers.push(modifier.clone());
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::AddModifier {
                iid: iid.clone(),
                modifier,
            });
        }
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
    pub fn bump_action(&mut self, action: &str, who: PlayerId) {
        let entry = self
            .action_counts
            .entry(action.to_string())
            .or_insert([0, 0]);
        let idx = match who {
            PlayerId::A => 0,
            PlayerId::B => 1,
        };
        entry[idx] += 1;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::BumpAction {
                action: action.to_string(),
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

    /// C.12: effective stats = printed X/Y + stored stat modifiers + static
    /// stat modifiers broadcast by on-board sources. Re-evaluated on every
    /// call (no caching). Returns (0, 0) for cards without printed stats.
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
        // STATIC.md Phase 1 + 2: iterate every potential static source —
        // on-board cards plus cards attached to them. If a source's static
        // matches this candidate via affects, add the declared modifier.
        for source_iid in self.static_source_iids() {
            if let Some((dx, dy)) = self.evaluate_static_stat_modifier(&source_iid, iid) {
                x += dx;
                y += dy;
            }
        }
        (x, y)
    }

    /// Phase 1 static evaluator: if `source_iid` is a card with a static
    /// stat-modifier whose `affects` predicate matches `target_iid`, returns
    /// the (x, y) delta. None otherwise.
    pub fn evaluate_static_stat_modifier(
        &self,
        source_iid: &InstanceId,
        target_iid: &InstanceId,
    ) -> Option<(i32, i32)> {
        let def = self.static_def_if_matches(source_iid, target_iid)?;
        Some((def.modifier_x, def.modifier_y))
    }

    /// Phase 2 static evaluator: if `source_iid` is a card with a static
    /// keyword-grant whose `affects` predicate matches `target_iid`, returns
    /// the granted (lowercase) keyword. None otherwise.
    pub fn evaluate_static_keyword_grant(
        &self,
        source_iid: &InstanceId,
        target_iid: &InstanceId,
    ) -> Option<&str> {
        let def = self.static_def_if_matches(source_iid, target_iid)?;
        def.modifier_keyword.as_deref()
    }

    /// Shared affects-predicate check. Returns the source's StaticDef iff
    /// `source_iid` has one AND its `condition` (if any) is satisfied AND
    /// `target_iid` matches its `affects` predicate.
    fn static_def_if_matches(
        &self,
        source_iid: &InstanceId,
        target_iid: &InstanceId,
    ) -> Option<&crate::card::StaticDef> {
        let source = self.card_pool.get(source_iid)?;
        let def = source.card.static_def.as_ref()?;
        let target = self.card_pool.get(target_iid)?;

        // Phase 2 condition gate: short-circuit before any affects logic.
        if let Some(cond) = &def.condition {
            if !self.evaluate_static_condition(source.controller, cond) {
                return None;
            }
        }

        let affects = &def.affects;
        if affects.exclude_self && source_iid == target_iid {
            return None;
        }
        // Scope check. AttachedHost requires that the source is in the
        // target's `attached` list (i.e., target IS the host of source).
        // SourceOnly requires target == source (the static targets itself).
        match affects.scope {
            crate::card::StaticScope::Board => {}
            crate::card::StaticScope::AttachedHost => {
                if !target.attached.iter().any(|x| x == source_iid) {
                    return None;
                }
            }
            crate::card::StaticScope::SourceOnly => {
                if source_iid != target_iid {
                    return None;
                }
            }
        }
        if let Some(ctrl) = affects.controller {
            let same_side = source.controller == target.controller;
            match ctrl {
                crate::card::StaticController::Owner if !same_side => return None,
                crate::card::StaticController::Opponent if same_side => return None,
                _ => {}
            }
        }
        if !affects.subtypes.is_empty() {
            let target_subs: Vec<String> = target
                .card
                .subtypes
                .iter()
                .map(|s| s.to_ascii_lowercase())
                .collect();
            if !affects.subtypes.iter().any(|s| target_subs.contains(s)) {
                return None;
            }
        }
        if !affects.colors.is_empty() {
            let target_colors: Vec<String> = target
                .card
                .colors
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if !affects.colors.iter().any(|c| target_colors.contains(c)) {
                return None;
            }
        }
        if let Some(k) = affects.kind {
            if target.card.kind != k {
                return None;
            }
        }
        Some(def)
    }

    /// Phase 2: evaluate a state-reading static condition against game
    /// state. `owner` is the source's controller — all `Owner*` variants
    /// look up against that player's zones.
    fn evaluate_static_condition(
        &self,
        owner: PlayerId,
        cond: &crate::card::StaticCondition,
    ) -> bool {
        match cond {
            crate::card::StaticCondition::OwnerGraveyardSize { min } => {
                self.player(owner).graveyard.len() >= *min
            }
            crate::card::StaticCondition::OwnerGraveyardNonCreatures { min } => {
                let non_creatures = self
                    .player(owner)
                    .graveyard
                    .iter()
                    .filter_map(|iid| self.card_pool.get(iid))
                    .filter(|inst| inst.card.kind != crate::card::CardType::Creature)
                    .count();
                non_creatures >= *min
            }
        }
    }

    /// Iterator yielding every potential static source: every on-board card,
    /// plus every card attached to an on-board card. Order is board-first,
    /// then attached-on-each-host. Used by `effective_stats` and
    /// `has_static_keyword` so attached sources (e.g., companion-bird with
    /// `scope = "attached_host"`) participate in static evaluation.
    fn static_source_iids(&self) -> Vec<InstanceId> {
        let mut out: Vec<InstanceId> = Vec::new();
        for board_iid in self.a.board.iter().chain(self.b.board.iter()) {
            out.push(board_iid.clone());
            if let Some(host) = self.card_pool.get(board_iid) {
                for att_iid in &host.attached {
                    out.push(att_iid.clone());
                }
            }
        }
        out
    }

    /// Phase 2: true if any on-board static source grants `keyword` to the
    /// card at `iid`. Mirrors `effective_stats`'s iteration shape.
    pub fn has_static_keyword(&self, iid: &InstanceId, keyword: &str) -> bool {
        for source_iid in self.static_source_iids() {
            if let Some(granted) = self.evaluate_static_keyword_grant(&source_iid, iid) {
                if granted == keyword {
                    return true;
                }
            }
        }
        false
    }

    /// Phase 2: full keyword check. Combines `CardInstance::has_keyword`
    /// (printed + intrinsic modifiers) with `has_static_keyword` (on-board
    /// static grants). Prefer this over the bare `CardInstance::has_keyword`
    /// wherever a `GameState` is reachable.
    pub fn has_keyword(&self, iid: &InstanceId, keyword: &str) -> bool {
        if let Some(inst) = self.card_pool.get(iid) {
            if inst.has_keyword(keyword) {
                return true;
            }
        }
        self.has_static_keyword(iid, keyword)
    }

    /// Phase 3: true if any on-board static source imposes `restriction`
    /// on the card at `iid`. Mirrors `has_static_keyword` iteration shape.
    pub fn has_restriction(&self, iid: &InstanceId, restriction: crate::card::Restriction) -> bool {
        for source_iid in self.static_source_iids() {
            if let Some(def) = self.static_def_if_matches(&source_iid, iid) {
                if def.restrictions.contains(&restriction) {
                    return true;
                }
            }
        }
        false
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
        s.card_pool.get_mut(&iid).unwrap().card = card_no_stats("instant", CardType::Spell);
        assert_eq!(s.effective_stats(&iid), (0, 0));
    }

    #[test]
    fn player_id_opponent_swaps() {
        assert_eq!(PlayerId::A.opponent(), PlayerId::B);
        assert_eq!(PlayerId::B.opponent(), PlayerId::A);
    }

    fn dummy_played(s: &GameState) -> StackItem {
        StackItem::PlayedCard {
            card: s.a.hand[0].clone(),
            controller: PlayerId::A,
            choices: super::super::PlayChoices::default(),
        }
    }

    fn dummy_played_for(card: InstanceId, controller: PlayerId) -> StackItem {
        StackItem::PlayedCard {
            card,
            controller,
            choices: super::super::PlayChoices::default(),
        }
    }

    #[test]
    fn open_window_sets_priority_and_chain() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let item = dummy_played(&s);
        s.open_response_window(item.clone()).unwrap();
        let p = s.priority.as_ref().unwrap();
        assert_eq!(p.chain, vec![item]);
        assert_eq!(p.next_to_act, s.active_player); // R.7
        assert_eq!(p.consecutive_passes, 0);
    }

    #[test]
    fn open_window_twice_errors() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let item = dummy_played(&s);
        s.open_response_window(item.clone()).unwrap();
        assert_eq!(
            s.open_response_window(item),
            Err(PriorityError::WindowAlreadyOpen),
        );
    }

    #[test]
    fn pass_priority_without_window_errors() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        assert_eq!(s.pass_priority(), Err(PriorityError::NoWindowOpen));
    }

    #[test]
    fn one_pass_hands_priority_to_opponent() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let item = dummy_played(&s);
        s.open_response_window(item).unwrap();
        // Opens with active (A); one pass hands to B.
        assert_eq!(s.pass_priority().unwrap(), None);
        let p = s.priority.as_ref().unwrap();
        assert_eq!(p.next_to_act, PlayerId::B);
        assert_eq!(p.consecutive_passes, 1);
    }

    #[test]
    fn two_passes_pop_and_close_when_chain_empties() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let item = dummy_played(&s);
        s.open_response_window(item.clone()).unwrap();
        assert_eq!(s.pass_priority().unwrap(), None);
        assert_eq!(s.pass_priority().unwrap(), Some(item));
        assert!(s.priority.is_none());
    }

    #[test]
    fn respond_pushes_and_flips_priority() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let item_a = dummy_played(&s);
        let item_b = dummy_played_for(s.b.hand[0].clone(), PlayerId::B);
        s.open_response_window(item_a.clone()).unwrap();
        s.pass_priority().unwrap(); // A → B
        s.respond_with(item_b.clone()).unwrap(); // B responds → A
        let p = s.priority.as_ref().unwrap();
        assert_eq!(p.chain, vec![item_a, item_b]);
        assert_eq!(p.next_to_act, PlayerId::A);
        assert_eq!(p.consecutive_passes, 0);
    }

    #[test]
    fn two_passes_with_two_items_pop_top_and_continue() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let item_a = dummy_played(&s);
        let item_b = dummy_played_for(s.b.hand[0].clone(), PlayerId::B);
        s.open_response_window(item_a.clone()).unwrap();
        s.pass_priority().unwrap();
        s.respond_with(item_b.clone()).unwrap();
        // Two passes → item_b resolves; window stays open with item_a as new top.
        s.pass_priority().unwrap();
        let popped = s.pass_priority().unwrap();
        assert_eq!(popped, Some(item_b));
        let p = s.priority.as_ref().unwrap();
        assert_eq!(p.chain, vec![item_a]);
        assert_eq!(p.next_to_act, s.active_player);
        assert_eq!(p.consecutive_passes, 0);
    }

    #[test]
    fn priority_state_round_trips_through_journal() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        s.journal = Some(crate::game::Journal::new());
        let snapshot = s.clone();
        let item = dummy_played(&s);
        let response = dummy_played_for(s.b.hand[0].clone(), PlayerId::B);
        s.open_response_window(item.clone()).unwrap();
        s.pass_priority().unwrap();
        s.respond_with(response).unwrap();
        s.journal.take().unwrap().rollback(&mut s);
        assert_eq!(s.priority, snapshot.priority);
    }

    #[test]
    fn counter_target_removes_specific_chain_item() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let item_a = dummy_played(&s);
        let b_card = s.b.hand[0].clone();
        let item_b = dummy_played_for(b_card.clone(), PlayerId::B);
        s.open_response_window(item_a.clone()).unwrap();
        s.pass_priority().unwrap();
        s.respond_with(item_b.clone()).unwrap();
        // Chain: [item_a, item_b]. Target item_a (the bottom) by its card id.
        let a_card = match &item_a {
            StackItem::PlayedCard { card, .. } => card.clone(),
        };
        let removed = s.counter_target(&a_card).unwrap();
        assert_eq!(removed, item_a);
        // item_b should still be on the chain.
        let p = s.priority.as_ref().unwrap();
        assert_eq!(p.chain.len(), 1);
        assert_eq!(p.chain[0], item_b);
        assert_eq!(p.next_to_act, s.active_player);
        assert_eq!(p.consecutive_passes, 0);
    }

    #[test]
    fn counter_target_returns_none_for_missing_target() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let item = dummy_played(&s);
        s.open_response_window(item).unwrap();
        assert_eq!(s.counter_target(&"nonexistent".to_string()), None);
    }

    #[test]
    fn legal_counter_targets_returns_chain_cards_in_order() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let a_card = s.a.hand[0].clone();
        let b_card = s.b.hand[0].clone();
        let item_a = dummy_played_for(a_card.clone(), PlayerId::A);
        let item_b = dummy_played_for(b_card.clone(), PlayerId::B);
        assert_eq!(s.legal_counter_targets(), Vec::<InstanceId>::new());
        s.open_response_window(item_a).unwrap();
        s.pass_priority().unwrap();
        s.respond_with(item_b).unwrap();
        assert_eq!(s.legal_counter_targets(), vec![a_card, b_card]);
    }

    fn make_anthem_source(s: &mut GameState, iid: &InstanceId, subtype: &str, dx: i32, dy: i32) {
        let inst = s.card_pool.get_mut(iid).unwrap();
        inst.card.subtypes.push(subtype.to_string());
        inst.card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec![subtype.to_ascii_lowercase()],
                colors: vec![],
                controller: Some(crate::card::StaticController::Owner),
                exclude_self: true,
                scope: crate::card::StaticScope::Board,
                kind: None,
            },
            modifier_x: dx,
            modifier_y: dy,
            modifier_keyword: None,
            condition: None,
            restrictions: Vec::new(),
        });
    }

    #[test]
    fn anthem_applies_to_matching_subtype_on_board() {
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        let anthem = s.a.hand[0].clone();
        let target = s.a.hand[1].clone();
        let unrelated = s.a.hand[2].clone();
        // Make target a human, unrelated a goblin.
        s.card_pool.get_mut(&target).unwrap().card.subtypes = vec!["human".into()];
        s.card_pool.get_mut(&unrelated).unwrap().card.subtypes = vec!["goblin".into()];
        // anthem source is a human anthem.
        make_anthem_source(&mut s, &anthem, "human", 1, 1);
        // Put all three on A's board.
        s.a.hand.retain(|i| i != &anthem && i != &target && i != &unrelated);
        s.a.board.push(anthem.clone());
        s.a.board.push(target.clone());
        s.a.board.push(unrelated.clone());

        // Target (human) gets boosted; unrelated (goblin) does not; source
        // doesn't self-boost.
        assert_eq!(s.effective_stats(&target), (2, 2));
        assert_eq!(s.effective_stats(&unrelated), (1, 1));
        assert_eq!(s.effective_stats(&anthem), (1, 1));
    }

    #[test]
    fn anthem_removed_when_source_leaves_board() {
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        let anthem = s.a.hand[0].clone();
        let target = s.a.hand[1].clone();
        s.card_pool.get_mut(&target).unwrap().card.subtypes = vec!["human".into()];
        make_anthem_source(&mut s, &anthem, "human", 1, 1);
        s.a.hand.retain(|i| i != &anthem && i != &target);
        s.a.board.push(anthem.clone());
        s.a.board.push(target.clone());
        assert_eq!(s.effective_stats(&target), (2, 2));
        // Move anthem to graveyard — boost evaporates.
        s.a.board.retain(|i| i != &anthem);
        s.a.graveyard.push(anthem);
        assert_eq!(s.effective_stats(&target), (1, 1));
    }

    #[test]
    fn attached_host_scope_grants_keyword_to_host() {
        // Companion-bird shape: a card with `scope = AttachedHost` +
        // `modifier_keyword = "flying"` grants flying to whatever host it's
        // attached to, and to nothing else.
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        let bird = s.a.hand[0].clone();
        let host = s.a.hand[1].clone();
        let bystander = s.a.hand[2].clone();
        // Bird = attached-host flying-granter.
        s.card_pool.get_mut(&bird).unwrap().card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec![],
                colors: vec![],
                controller: None,
                exclude_self: false,
                scope: crate::card::StaticScope::AttachedHost,
                kind: None,
            },
            modifier_x: 0,
            modifier_y: 0,
            modifier_keyword: Some("flying".into()),
            condition: None,
            restrictions: Vec::new(),
        });
        // Move host + bystander to board.
        s.a.hand.retain(|i| i != &bird && i != &host && i != &bystander);
        s.a.board.push(host.clone());
        s.a.board.push(bystander.clone());
        // Attach bird to host (companion-bird arrives as a HAND payment).
        s.add_attached(&host, &bird);
        // Host gains flying via the AttachedHost static. Bystander does not.
        assert!(s.has_keyword(&host, "flying"));
        assert!(!s.has_keyword(&bystander, "flying"));
    }

    #[test]
    fn attached_host_scope_does_not_grant_when_unattached() {
        // Same source card, but the bird is on the BOARD (not attached) —
        // the AttachedHost predicate has no host to point at.
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        let bird = s.a.hand[0].clone();
        let target = s.a.hand[1].clone();
        s.card_pool.get_mut(&bird).unwrap().card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec![],
                colors: vec![],
                controller: None,
                exclude_self: false,
                scope: crate::card::StaticScope::AttachedHost,
                kind: None,
            },
            modifier_x: 0,
            modifier_y: 0,
            modifier_keyword: Some("flying".into()),
            condition: None,
            restrictions: Vec::new(),
        });
        s.a.hand.retain(|i| i != &bird && i != &target);
        s.a.board.push(bird);
        s.a.board.push(target.clone());
        assert!(!s.has_keyword(&target, "flying"));
    }

    #[test]
    fn condition_gate_blocks_static_until_graveyard_threshold() {
        // Ossuary-shape: static fires only when owner's graveyard has 5+ cards.
        let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
        let source = s.a.hand[0].clone();
        let target = s.a.hand[1].clone();
        s.card_pool.get_mut(&target).unwrap().card.kind = crate::card::CardType::Creature;
        s.card_pool.get_mut(&source).unwrap().card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec![],
                colors: vec![],
                controller: Some(crate::card::StaticController::Owner),
                exclude_self: true,
                scope: crate::card::StaticScope::Board,
                kind: Some(crate::card::CardType::Creature),
            },
            modifier_x: 1,
            modifier_y: 1,
            modifier_keyword: Some("flying".into()),
            condition: Some(crate::card::StaticCondition::OwnerGraveyardSize { min: 5 }),
            restrictions: Vec::new(),
        });
        s.a.hand.retain(|i| i != &source && i != &target);
        s.a.board.push(source);
        s.a.board.push(target.clone());

        // Empty graveyard: condition fails, no boost, no flying.
        assert_eq!(s.effective_stats(&target), (1, 1));
        assert!(!s.has_keyword(&target, "flying"));

        // Move 5 cards from A's deck to graveyard.
        let to_mill: Vec<_> = s.a.deck.iter().take(5).cloned().collect();
        for iid in to_mill {
            s.a.deck.retain(|x| x != &iid);
            s.a.graveyard.push(iid);
        }
        assert_eq!(s.a.graveyard.len(), 5);

        // Now the condition is met: +1/+1 + flying applies.
        assert_eq!(s.effective_stats(&target), (2, 2));
        assert!(s.has_keyword(&target, "flying"));
    }

    #[test]
    fn condition_non_creatures_counts_only_non_creature_kinds() {
        // Wandering-wizard-shape: the static counts NON-creature cards in
        // graveyard. A graveyard full of creatures should NOT trigger it.
        let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
        let wizard = s.a.hand[0].clone();
        s.card_pool.get_mut(&wizard).unwrap().card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec![],
                colors: vec![],
                controller: None,
                exclude_self: false,
                scope: crate::card::StaticScope::SourceOnly,
                kind: None,
            },
            modifier_x: 0,
            modifier_y: 0,
            modifier_keyword: Some("flying".into()),
            condition: Some(crate::card::StaticCondition::OwnerGraveyardNonCreatures { min: 4 }),
            restrictions: Vec::new(),
        });
        s.a.hand.retain(|i| i != &wizard);
        s.a.board.push(wizard.clone());

        // Fill graveyard with creatures: deck_of() makes every card a creature.
        let to_mill: Vec<_> = s.a.deck.iter().take(6).cloned().collect();
        for iid in to_mill {
            s.a.deck.retain(|x| x != &iid);
            s.a.graveyard.push(iid);
        }
        // Graveyard has 6 cards but they're all creatures → non-creature count
        // is 0 → flying NOT granted.
        assert_eq!(s.a.graveyard.len(), 6);
        assert!(!s.has_keyword(&wizard, "flying"));

        // Flip 4 of them to Spell — non-creature count hits 4 → flying ON.
        let gy = s.a.graveyard.clone();
        for iid in gy.iter().take(4) {
            s.card_pool.get_mut(iid).unwrap().card.kind = crate::card::CardType::Spell;
        }
        assert!(s.has_keyword(&wizard, "flying"));
    }

    #[test]
    fn source_only_scope_targets_only_the_source() {
        // SourceOnly scope: the static targets the source card itself, not
        // other on-board cards even if they match other predicates.
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        let wizard = s.a.hand[0].clone();
        let other = s.a.hand[1].clone();
        s.card_pool.get_mut(&wizard).unwrap().card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec![],
                colors: vec![],
                controller: None,
                exclude_self: false,
                scope: crate::card::StaticScope::SourceOnly,
                kind: None,
            },
            modifier_x: 0,
            modifier_y: 0,
            modifier_keyword: Some("flying".into()),
            condition: None,
            restrictions: Vec::new(),
        });
        s.a.hand.retain(|i| i != &wizard && i != &other);
        s.a.board.push(wizard.clone());
        s.a.board.push(other.clone());
        assert!(s.has_keyword(&wizard, "flying"));
        assert!(!s.has_keyword(&other, "flying"));
    }

    #[test]
    fn restriction_cannot_attack_propagates_to_opponent_insects() {
        // Flesh-eating-plant shape: opponent's insects get CannotAttack.
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        let plant = s.b.hand[0].clone();
        let opp_insect = s.a.hand[0].clone();
        let own_insect = s.b.hand[1].clone();
        s.card_pool.get_mut(&opp_insect).unwrap().card.subtypes = vec!["insect".into()];
        s.card_pool.get_mut(&own_insect).unwrap().card.subtypes = vec!["insect".into()];
        s.card_pool.get_mut(&plant).unwrap().card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec!["insect".into()],
                colors: vec![],
                controller: Some(crate::card::StaticController::Opponent),
                exclude_self: false,
                scope: crate::card::StaticScope::Board,
                kind: None,
            },
            modifier_x: 0,
            modifier_y: 0,
            modifier_keyword: None,
            condition: None,
            restrictions: vec![
                crate::card::Restriction::CannotAttack,
                crate::card::Restriction::CannotBeCostPaid,
            ],
        });
        s.b.hand.retain(|i| i != &plant && i != &own_insect);
        s.a.hand.retain(|i| i != &opp_insect);
        s.b.board.push(plant);
        s.b.board.push(own_insect.clone());
        s.a.board.push(opp_insect.clone());

        // Plant is on B's board; A's insect is opponent's insect → restricted.
        // B's own insect is NOT restricted (controller filter = "opponent" of
        // the source = A; B's insect is on the same side as the source).
        assert!(s.has_restriction(&opp_insect, crate::card::Restriction::CannotAttack));
        assert!(s.has_restriction(&opp_insect, crate::card::Restriction::CannotBeCostPaid));
        assert!(!s.has_restriction(&own_insect, crate::card::Restriction::CannotAttack));
        assert!(!s.has_restriction(&own_insect, crate::card::Restriction::CannotBeCostPaid));
    }

    #[test]
    fn restriction_cannot_attack_blocks_declare_attacker() {
        use crate::card::CardType;
        // End-to-end: declare_attacker errors out when the would-be attacker
        // has the CannotAttack restriction.
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        let plant = s.b.hand[0].clone();
        let attacker = s.a.hand[0].clone();
        s.card_pool.get_mut(&attacker).unwrap().card.subtypes = vec!["insect".into()];
        s.card_pool.get_mut(&attacker).unwrap().card.kind = CardType::Creature;
        s.card_pool.get_mut(&plant).unwrap().card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec!["insect".into()],
                colors: vec![],
                controller: Some(crate::card::StaticController::Opponent),
                exclude_self: false,
                scope: crate::card::StaticScope::Board,
                kind: None,
            },
            modifier_x: 0,
            modifier_y: 0,
            modifier_keyword: None,
            condition: None,
            restrictions: vec![crate::card::Restriction::CannotAttack],
        });
        s.b.hand.retain(|i| i != &plant);
        s.a.hand.retain(|i| i != &attacker);
        s.b.board.push(plant);
        s.a.board.push(attacker.clone());

        // Set up combat phase for player A (the would-be attacker's controller).
        s.active_player = PlayerId::A;
        s.phase = crate::game::Phase::Combat;
        s.card_pool.get_mut(&attacker).unwrap().summoning_sick = false;
        let err = s.declare_attacker(&attacker, None).unwrap_err();
        assert_eq!(err, crate::game::combat::CombatError::AttackerForbiddenByRestriction);
    }

    #[test]
    fn two_anthems_stack() {
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        let anthem_a = s.a.hand[0].clone();
        let anthem_b = s.a.hand[1].clone();
        let target = s.a.hand[2].clone();
        s.card_pool.get_mut(&target).unwrap().card.subtypes = vec!["human".into()];
        make_anthem_source(&mut s, &anthem_a, "human", 1, 1);
        make_anthem_source(&mut s, &anthem_b, "human", 2, 0);
        s.a.hand.retain(|i| i != &anthem_a && i != &anthem_b && i != &target);
        s.a.board.push(anthem_a);
        s.a.board.push(anthem_b);
        s.a.board.push(target.clone());
        // Both anthems are humans too (via make_anthem_source push), but
        // exclude_self skips self. They DO boost each other though, and the
        // target. Target: 1 + 1 + 2 = 4 / 1 + 1 + 0 = 2.
        assert_eq!(s.effective_stats(&target), (4, 2));
    }

    #[test]
    fn opponent_controlled_anthem_does_not_affect_owner_filtered() {
        let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
        // B has an "owner" anthem for humans — only B's humans should be
        // boosted, not A's.
        let b_anthem = s.b.hand[0].clone();
        let a_human = s.a.hand[0].clone();
        s.card_pool.get_mut(&a_human).unwrap().card.subtypes = vec!["human".into()];
        make_anthem_source(&mut s, &b_anthem, "human", 1, 1);
        s.b.hand.retain(|i| i != &b_anthem);
        s.a.hand.retain(|i| i != &a_human);
        s.b.board.push(b_anthem);
        s.a.board.push(a_human.clone());
        // A's human is on board, B's anthem is on board, but controller
        // filter is "owner" — B's anthem boosts only B's humans.
        assert_eq!(s.effective_stats(&a_human), (1, 1));
    }

    #[test]
    fn playable_responses_filters_to_zero_cost_instants() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        // a_hand[0] is a creature by default — not a response candidate.
        // Mutate a_hand[1] into a zero-cost instant.
        let inst = s.a.hand[1].clone();
        let card = s.card_pool.get_mut(&inst).unwrap();
        card.card.kind = crate::card::CardType::Spell;
        card.card.timing = Some(crate::card::Timing::Instant);
        card.card.cost = vec![];
        // Mutate a_hand[2] into a sorcery — should NOT be returned.
        let sorc = s.a.hand[2].clone();
        let card2 = s.card_pool.get_mut(&sorc).unwrap();
        card2.card.kind = crate::card::CardType::Spell;
        card2.card.timing = Some(crate::card::Timing::Sorcery);
        card2.card.cost = vec![];
        let candidates = s.playable_responses(PlayerId::A);
        assert!(candidates.contains(&inst));
        assert!(!candidates.contains(&sorc));
    }
}
