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

    /// Numeric index for array lookups indexed by player. `A → 0`,
    /// `B → 1`. Used by callers that hold `[T; 2]` keyed by player
    /// (e.g., per-player AI config in the sim).
    pub fn index(self) -> usize {
        match self {
            PlayerId::A => 0,
            PlayerId::B => 1,
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
    pub damage: f32,               // B.7–B.8 accumulated (fractional since the f32-stats refactor)
    pub summoning_sick: bool,      // B.3 (cleared at start of controller's turn)
    /// True iff this card was declared as an attacker during the
    /// current turn. Cleared at the start of each turn. Used by
    /// activated abilities like vigilant-human's `T: if this creature
    /// attacked this turn, draw a card`. Distinct from the global
    /// `creature_attacked_this_turn` (which only tracks whether ANY
    /// creature attacked, not which one).
    #[serde(default)]
    pub attacked_this_turn: bool,
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
            match m {
                Modifier::GainsFlying if keyword == "flying" => return true,
                Modifier::GainsVigilance | Modifier::EotGainsVigilance
                    if keyword == "vigilance" =>
                {
                    return true
                }
                Modifier::GainsHaste | Modifier::EotGainsHaste if keyword == "haste" => {
                    return true
                }
                _ => {}
            }
        }
        false
    }
}

/// Continuous modifiers applied to a card's effective state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Modifier {
    /// e.g., +1/+1
    StatBoost { x: f32, y: f32 },
    /// e.g., Companion Bird grants flying while attached
    GainsFlying,
    /// e.g., Flesh-eating Plant
    CantAttack,
    /// Temporary stat boost that expires at the end of the current turn.
    /// Used for cards like unblockable-human's `+2/+0 until end of turn`
    /// trigger and bring-down's `-3/-3 until end of turn` (once that
    /// migrates off the damage-proxy approximation).
    EotStatBoost { x: f32, y: f32 },
    /// Granted vigilance (e.g., via attached card or activation). Mirrors
    /// `GainsFlying` for the vigilance keyword.
    GainsVigilance,
    /// Granted vigilance until end of turn. Cleared by
    /// `clear_eot_modifiers` alongside `EotStatBoost`. Used by the white
    /// monkey's `2 hand: creatures you control get +2/+2 and vigilance
    /// until end of turn` activation.
    EotGainsVigilance,
    /// Granted haste (e.g., via activation rider). Mirrors `GainsFlying`.
    GainsHaste,
    /// Granted haste until end of turn. Used by the red monkey's "deal
    /// 2 damage to target; if it survives, that creature gains haste
    /// until end of turn" rider.
    EotGainsHaste,
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
    /// Transient X value for the activation currently resolving. Set
    /// by `activate_ability` before firing the handler; cleared after.
    /// Exposed to Lua handlers as `game.x_value()`. None outside of an
    /// active X-cost activation. Not journaled — the value is reset by
    /// each activate_ability call, not state-evolving on its own.
    #[serde(skip, default)]
    pub current_activation_x: Option<i32>,
    /// Queue of pending extra turns. When the End phase advances and
    /// this queue is non-empty, the front of the queue becomes the next
    /// active player instead of `active_player.opponent()`. Powers
    /// "target player takes an extra turn" effects. Multiple entries
    /// stack: each consumed in FIFO order; opponent only resumes when
    /// the queue is empty.
    #[serde(default)]
    pub extra_turns_pending: Vec<PlayerId>,
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
            current_activation_x: None,
            extra_turns_pending: Vec::new(),
        }
    }

    /// Internal: returns whichever journal should receive the next mutation.
    /// Preview journal wins if open (so previews can be cleanly rolled back
    /// without polluting the replay journal). Falls back to replay journal,
    /// which accumulates only committed mutations.
    pub fn active_journal(&mut self) -> Option<&mut super::Journal> {
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

    pub fn set_damage(&mut self, iid: &InstanceId, damage: f32) {
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

    pub fn set_attacked_this_turn(&mut self, iid: &InstanceId, attacked: bool) {
        let Some(inst) = self.card_pool.get_mut(iid) else {
            return;
        };
        let was = inst.attacked_this_turn;
        if was == attacked {
            return;
        }
        inst.attacked_this_turn = attacked;
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::SetAttackedThisTurn {
                iid: iid.clone(),
                was,
                now: attacked,
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
        // RULES P.33: a countered cast moves to GRAVEYARD (the cast
        // card already left HAND at announce-time; we need to put it
        // somewhere, and the resolution didn't fire).
        let StackItem::PlayedCard { card, controller, .. } = &removed;
        self.add_to_zone(card, *controller, Zone::Graveyard);
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
        // RULES P.33: countered cast moves to GRAVEYARD.
        let StackItem::PlayedCard { card, controller, .. } = &removed;
        self.add_to_zone(card, *controller, Zone::Graveyard);
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
                if hand.len() <= hand_need || deck_len < mill_need || gy_len < gy_need {
                    return false;
                }
                // RULES P.12a: GY-source cost component with non-empty
                // cast colors requires at least one color-matching card
                // in GY. Without this, response-pick burns rolls on
                // casts play_card refuses with NoGraveyardPaymentForColor
                // → priority window spin.
                if gy_need > 0 {
                    let cast_colors: std::collections::BTreeSet<String> = inst
                        .card
                        .colors
                        .iter()
                        .map(|c| c.to_ascii_lowercase())
                        .collect();
                    if !cast_colors.is_empty() {
                        let has_anchor = p.graveyard.iter().any(|gid| {
                            self.card_pool
                                .get(gid)
                                .map(|i| {
                                    i.card
                                        .colors
                                        .iter()
                                        .any(|c| cast_colors.contains(&c.to_ascii_lowercase()))
                                })
                                .unwrap_or(false)
                        });
                        if !has_anchor {
                            return false;
                        }
                    }
                }
                // RULES P.32: refuse if the card declares a target category
                // and no legal target exists. Without this, counterspell
                // (target = "chain") is a candidate when the chain is empty;
                // play_card refuses with CastValidateFailed and the
                // priority window can't advance.
                if let Some(target) = inst.card.target {
                    if !self.is_target_legal(target) {
                        return false;
                    }
                }
                true
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

    /// Strip all `Modifier::EotStatBoost` entries from every card_pool
    /// instance. Called at end-of-turn so "until end of turn" pump effects
    /// (unblockable-human's +2/+0, bring-down's -3/-3 when migrated) expire
    /// on the right boundary. Journaled per-card so rollback restores the
    /// pre-clear state exactly.
    pub fn clear_eot_modifiers(&mut self) {
        // Snapshot the iid list to avoid borrow issues. card_pool is a
        // BTreeMap so iteration order is stable.
        let iids: Vec<InstanceId> = self.card_pool.keys().cloned().collect();
        for iid in iids {
            let Some(inst) = self.card_pool.get(&iid) else {
                continue;
            };
            // Capture (pos, modifier) for each EOT entry before mutation.
            let removed: Vec<(usize, Modifier)> = inst
                .modifiers
                .iter()
                .enumerate()
                .filter_map(|(i, m)| match m {
                    Modifier::EotStatBoost { .. }
                    | Modifier::EotGainsVigilance
                    | Modifier::EotGainsHaste => Some((i, m.clone())),
                    _ => None,
                })
                .collect();
            if removed.is_empty() {
                continue;
            }
            if let Some(inst_mut) = self.card_pool.get_mut(&iid) {
                inst_mut.modifiers.retain(|m| {
                    !matches!(
                        m,
                        Modifier::EotStatBoost { .. }
                            | Modifier::EotGainsVigilance
                            | Modifier::EotGainsHaste
                    )
                });
            }
            if let Some(j) = self.active_journal() {
                j.push(super::JournalEntry::ClearEotModifiers {
                    iid: iid.clone(),
                    removed,
                });
            }
        }
    }

    /// Append an iid to host's attached vec, journaling the addition.
    /// RULES P.32: built-in legality check for a declared cast target
    /// category. Returns true iff at least one legal target exists in
    /// the current state. Pure read — no Lua, no mutation.
    pub fn is_target_legal(&self, target: crate::card::Target) -> bool {
        match target {
            crate::card::Target::Chain => self
                .priority
                .as_ref()
                .map(|p| !p.chain.is_empty())
                .unwrap_or(false),
        }
    }

    /// Find the host iid that has `attached` in its attached vec.
    /// Returns None if `attached` isn't attached to anything in the pool.
    pub fn host_of(&self, attached: &InstanceId) -> Option<InstanceId> {
        for (host_iid, host) in &self.card_pool {
            if host.attached.iter().any(|x| x == attached) {
                return Some(host_iid.clone());
            }
        }
        None
    }

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

    /// Insert an iid at position 0 of a player's zone, journaling the
    /// insertion. Primary use: putting a card on TOP of the deck (V.1
    /// considers index 0 the top). Sprout-style cantrips need this.
    pub fn add_to_zone_top(&mut self, iid: &InstanceId, owner: PlayerId, zone: Zone) {
        let p = self.player_mut(owner);
        let zone_vec = match zone {
            Zone::Board => &mut p.board,
            Zone::Hand => &mut p.hand,
            Zone::Deck => &mut p.deck,
            Zone::Graveyard => &mut p.graveyard,
            Zone::Exile => &mut p.exile,
        };
        zone_vec.insert(0, iid.clone());
        if let Some(j) = self.active_journal() {
            j.push(super::JournalEntry::AddToZoneTop {
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
                damage: 0.0,
                summoning_sick: false,
                attacked_this_turn: false,
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
    pub fn effective_stats(&self, iid: &InstanceId) -> (f32, f32) {
        let Some(inst) = self.card_pool.get(iid) else {
            return (0.0, 0.0);
        };
        let (mut x, mut y) = inst.card.stats.map(|s| (s.x, s.y)).unwrap_or((0.0, 0.0));
        for m in &inst.modifiers {
            match m {
                Modifier::StatBoost { x: dx, y: dy } => {
                    x += dx;
                    y += dy;
                }
                Modifier::EotStatBoost { x: dx, y: dy } => {
                    x += dx;
                    y += dy;
                }
                _ => {}
            }
        }
        // STATIC.md Phase 1 + 2: iterate every potential static source —
        // on-board cards plus cards attached to them. If a source's static
        // matches this candidate via affects, add the declared modifier.
        for source_iid in self.static_source_iids() {
            if let Some((dx, dy)) = self.evaluate_static_stat_modifier(source_iid, iid) {
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
    ) -> Option<(f32, f32)> {
        let def = self.static_def_if_matches(source_iid, target_iid)?;
        let source = self.card_pool.get(source_iid)?;
        let dx = self.resolve_modifier_value(source, &def.modifier_x);
        let dy = self.resolve_modifier_value(source, &def.modifier_y);
        Some((dx, dy))
    }

    /// Phase 1.5: resolve a `ModifierValue` against the source's current
    /// state. Counts walk the source's attached list every call, so the
    /// returned value tracks attached-set changes automatically — no
    /// snapshot leak. Returns f32 because stat modifiers are applied
    /// to f32 X/Y since the fractional-stats refactor; ModifierValue
    /// inputs are still integer (whole-card counts), the cast happens
    /// at this boundary.
    fn resolve_modifier_value(
        &self,
        source: &CardInstance,
        mv: &crate::card::ModifierValue,
    ) -> f32 {
        match mv {
            crate::card::ModifierValue::Fixed(n) => *n as f32,
            crate::card::ModifierValue::AttachedCount => source.attached.len() as f32,
            crate::card::ModifierValue::AttachedCountByColor(color) => {
                let needle = color.to_ascii_lowercase();
                source
                    .attached
                    .iter()
                    .filter(|aid| {
                        self.card_pool
                            .get(*aid)
                            .map(|c| {
                                c.card
                                    .colors
                                    .iter()
                                    .any(|col| col.eq_ignore_ascii_case(&needle))
                            })
                            .unwrap_or(false)
                    })
                    .count() as f32
            }
            crate::card::ModifierValue::AttachedCountByKind(kind) => source
                .attached
                .iter()
                .filter(|aid| {
                    self.card_pool
                        .get(*aid)
                        .map(|c| c.card.kind == *kind)
                        .unwrap_or(false)
                })
                .count() as f32,
            crate::card::ModifierValue::AttachedCountScaled(n) => {
                (source.attached.len() as f32) * (*n as f32)
            }
            crate::card::ModifierValue::BoardCount => {
                // RULES C.16: count each BOARD card as 1; attached do not contribute.
                (self.a.board.len() + self.b.board.len()) as f32
            }
            crate::card::ModifierValue::HandCount => {
                (self.a.hand.len() + self.b.hand.len()) as f32
            }
        }
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
            if !self.evaluate_static_condition(source.controller, source_iid, cond) {
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
        if let Some(kw) = &affects.has_keyword {
            // Cycle guard: only check INTRINSIC keywords (printed + intrinsic
            // modifiers), not static-granted. Otherwise this matcher would
            // recurse: static_def_if_matches → has_keyword → has_static_keyword
            // → static_def_if_matches → ... A future stratified evaluator
            // could handle the static-on-static case; for now the corpus
            // doesn't need it.
            if !target.has_keyword(kw) {
                return None;
            }
        }
        Some(def)
    }

    /// Phase 2: evaluate a state-reading static condition against game
    /// state. `owner` is the source's controller — `Owner*` variants
    /// look up against that player's zones. `source_iid` is the static
    /// source itself — variants that read its attached / position
    /// (e.g., `DeckTopSymbolMatchesAttached`) consult it.
    fn evaluate_static_condition(
        &self,
        owner: PlayerId,
        source_iid: &InstanceId,
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
            crate::card::StaticCondition::DeckTopSymbolMatchesAttached => {
                let top_symbols = self.effective_top_of_deck_symbols(owner);
                if top_symbols.is_empty() {
                    return false;
                }
                let Some(source) = self.card_pool.get(source_iid) else {
                    return false;
                };
                for att_iid in &source.attached {
                    if let Some(att) = self.card_pool.get(att_iid) {
                        for sym in &att.card.symbols {
                            if top_symbols.iter().any(|t| t == sym) {
                                return true;
                            }
                        }
                    }
                }
                false
            }
        }
    }

    /// V.8: a transparent card on top of a DECK reveals the symbols of
    /// the card immediately below it. Recursively skips transparent
    /// cards from the top until an opaque card is found and returns
    /// that card's symbols. Returns empty if the deck is empty or
    /// every card in it is transparent.
    pub fn effective_top_of_deck_symbols(&self, player: PlayerId) -> Vec<String> {
        for iid in &self.player(player).deck {
            if let Some(inst) = self.card_pool.get(iid) {
                let is_transparent = inst.card.frame.as_deref() == Some("transparent");
                if !is_transparent {
                    return inst.card.symbols.clone();
                }
            }
        }
        Vec::new()
    }

    /// Iterator yielding every potential static source: every on-board card,
    /// plus every card attached to an on-board card. Order is board-first,
    /// then attached-on-each-host. Used by `effective_stats` and
    /// `has_static_keyword` so attached sources (e.g., companion-bird with
    /// `scope = "attached_host"`) participate in static evaluation.
    /// Iterator over every potential static source — on-board cards plus
    /// every card in those cards' `attached` lists, FILTERED to entries
    /// that have a `static_def`. Most board cards have no static, and
    /// pre-filtering here saves the downstream `static_def_if_matches`
    /// from doing a card_pool lookup + None branch per call. Hot path:
    /// called from effective_stats, has_static_keyword, has_restriction,
    /// and cost_reduction, multiple times per AI decision.
    fn static_source_iids(&self) -> impl Iterator<Item = &InstanceId> + '_ {
        self.a
            .board
            .iter()
            .chain(self.b.board.iter())
            .flat_map(move |board_iid| {
                let host = self.card_pool.get(board_iid);
                let attached = host
                    .map(|h| h.attached.iter())
                    .into_iter()
                    .flatten();
                std::iter::once(board_iid).chain(attached)
            })
            .filter(move |iid| {
                self.card_pool
                    .get(*iid)
                    .map(|i| i.card.static_def.is_some())
                    .unwrap_or(false)
            })
    }

    /// Phase 2: true if any on-board static source grants `keyword` to the
    /// card at `iid`. Mirrors `effective_stats`'s iteration shape.
    pub fn has_static_keyword(&self, iid: &InstanceId, keyword: &str) -> bool {
        for source_iid in self.static_source_iids() {
            if let Some(granted) = self.evaluate_static_keyword_grant(source_iid, iid) {
                if granted == keyword {
                    return true;
                }
            }
        }
        false
    }

    /// Effective colors per RULES C.5 + the static color-grant system.
    /// Union of the card's printed colors with every `granted_colors`
    /// entry from active statics whose `affects` predicate matches.
    /// Case-insensitive deduped (lowercase). Used by P.7a identity
    /// matching, jewel pitch (P.24), and the Lua `game.card(iid).colors`
    /// surface. The static-affects matcher itself uses printed colors
    /// only (intrinsic-only check) to avoid recursion.
    /// C.14: does `iid`'s printed color set include `transparent`?
    /// Single source of truth for transparency checks — payment
    /// validation, attach validation, and the `game.attach` Lua API all
    /// consult this. Uses *printed* colors only (not effective), to
    /// match how C.14 reads: transparency is a physical property of
    /// the card, not something a static effect can grant or remove.
    pub fn is_transparent(&self, iid: &InstanceId) -> bool {
        self.card_pool
            .get(iid)
            .map(|i| i.card.frame.as_deref() == Some("transparent"))
            .unwrap_or(false)
    }

    pub fn effective_colors(&self, iid: &InstanceId) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(inst) = self.card_pool.get(iid) {
            for c in &inst.card.colors {
                let lc = c.to_ascii_lowercase();
                if !out.iter().any(|x| x == &lc) {
                    out.push(lc);
                }
            }
        }
        for source_iid in self.static_source_iids() {
            if let Some(def) = self.static_def_if_matches(source_iid, iid) {
                for c in &def.granted_colors {
                    let lc = c.to_ascii_lowercase();
                    if !out.iter().any(|x| x == &lc) {
                        out.push(lc);
                    }
                }
            }
        }
        out
    }

    /// Effective face per the card-surface system. Union of the card's
    /// printed face attributes with every `granted_face` entry from
    /// active statics whose `affects` predicate matches. Parallel shape
    /// to `effective_colors`. Used by the Lua `game.card(iid).face`
    /// surface so handlers can read host-granted face (e.g., a creature
    /// with a GFP attached has `face` including `"glow"`).
    pub fn effective_face(&self, iid: &InstanceId) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(inst) = self.card_pool.get(iid) {
            for f in &inst.card.face {
                let lc = f.to_ascii_lowercase();
                if !out.iter().any(|x| x == &lc) {
                    out.push(lc);
                }
            }
        }
        for source_iid in self.static_source_iids() {
            if let Some(def) = self.static_def_if_matches(source_iid, iid) {
                for f in &def.granted_face {
                    let lc = f.to_ascii_lowercase();
                    if !out.iter().any(|x| x == &lc) {
                        out.push(lc);
                    }
                }
            }
        }
        out
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
            if let Some(def) = self.static_def_if_matches(source_iid, iid) {
                if def.restrictions.contains(&restriction) {
                    return true;
                }
            }
        }
        false
    }

    /// Phase 3 (activated abilities): the total number of activations
    /// available on `iid` — its printed `card.activated` entries plus
    /// any granted by matching static abilities. Used by the sim AI's
    /// activation pass and by `activation_at` for index resolution.
    pub fn activation_count(&self, iid: &InstanceId) -> usize {
        let printed = self
            .card_pool
            .get(iid)
            .map(|i| i.card.activated.len())
            .unwrap_or(0);
        let granted = self
            .static_source_iids()
            .filter(|src| {
                self.static_def_if_matches(src, iid)
                    .and_then(|def| def.granted_activated.as_ref())
                    .is_some()
            })
            .count();
        printed + granted
    }

    /// Returns the activated ability at `idx` on `iid`. Indices < the
    /// number of printed activations resolve to `card.activated[idx]`;
    /// higher indices walk static-granted abilities in
    /// `static_source_iids()` order. Returns `None` past the total.
    pub fn activation_at(&self, iid: &InstanceId, idx: usize) -> Option<&crate::card::ActivatedAbility> {
        let inst = self.card_pool.get(iid)?;
        let printed_count = inst.card.activated.len();
        if idx < printed_count {
            return inst.card.activated.get(idx);
        }
        let mut remaining = idx - printed_count;
        for src in self.static_source_iids() {
            if let Some(def) = self.static_def_if_matches(src, iid) {
                if let Some(granted) = def.granted_activated.as_ref() {
                    if remaining == 0 {
                        return Some(granted);
                    }
                    remaining -= 1;
                }
            }
        }
        None
    }

    /// Phase 3.5: total cost reduction applied to casting `iid` from
    /// `source`-typed components, summed across all matching on-board
    /// static sources. Used by play_card to reduce per-source cost
    /// requirements before validation; per P.20 the caller clamps each
    /// resulting amount to a minimum of 0.
    pub fn cost_reduction(
        &self,
        iid: &InstanceId,
        source: crate::card::CostSource,
    ) -> i32 {
        let mut total = 0i32;
        for source_iid in self.static_source_iids() {
            if let Some(def) = self.static_def_if_matches(source_iid, iid) {
                for m in &def.cost_modifiers {
                    if m.source == source {
                        total += m.amount;
                    }
                }
            }
        }
        total
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
