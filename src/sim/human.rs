//! Human-driven oracle for `tsot serve`. The engine runs synchronously on
//! one thread; this module is the bridge a frontend (HTTP server, future
//! wasm host) uses to drive decisions through a channel pair.
//!
//! Shape: each decision site that previously called `select_attackers` /
//! `pick_blocks` / `pick_random_playable_in_hand` / `ChoiceOracle::*` now
//! dispatches on `AiKind`. The `AiKind::Human(Arc<HumanInterface>)` arm
//! sends a [`HumanPrompt`] to the frontend and blocks on a [`HumanAction`]
//! coming back.
//!
//! Phase A1: only the card-pick site is wired. Combat (attackers / blocks)
//! still goes through the heuristic regardless of AiKind — that's the
//! next refactor.
//!
//! Lua choices currently route through `ChoiceOracle` and are still
//! answered by `RandomOracle` even in human games — wiring them to the
//! human is part of Phase A4.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::choice::{
    ChoiceOracle, ChooseCardRequest, ChooseIntRequest, ChoosePlayerRequest, TargetIntent,
};
use crate::game::{GameState, InstanceId, PlayerId};

use super::ai::PickKindFilter;
use super::snapshot::{build_state_view, StateView};

/// Result of [`HumanInterface::main_phase_choice`].
#[derive(Debug, Clone)]
pub enum MainPhaseChoice {
    Pass,
    Play(InstanceId),
    Activate {
        iid: InstanceId,
        ability_index: usize,
        x: Option<i32>,
    },
}

/// One activatable ability slot for the human's main-phase prompt.
/// `text` is the human-readable ability text (from `card.activated[idx].text`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActivationOption {
    pub iid: InstanceId,
    pub card_name: String,
    pub ability_index: usize,
    pub text: String,
    pub needs_x: bool,
}

/// What the engine is asking the human to decide. One variant per
/// decision point. Future variants (attackers, blocks, Lua choices)
/// land here as Phase A progresses.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind")]
pub enum HumanPrompt {
    /// Main-phase prompt: human may play a card from hand, activate
    /// an ability on a board card, or pass. `candidates` is from
    /// [`super::ai::enumerate_playable_in_hand`] (hand cards the
    /// player can afford to cast under `kind_filter`). `activations`
    /// lists every ability on the player's board that's currently
    /// activatable (passes `can_activate`). The human-only main-phase
    /// loop in `run_game_continue` keeps iterating this prompt until
    /// the human passes.
    PickCard {
        state: StateView,
        player: PlayerId,
        candidates: Vec<InstanceId>,
        kind_filter: PickKindFilter,
        activations: Vec<ActivationOption>,
    },
    /// Combat-step attacker declaration. `eligible` is from
    /// [`super::ai::eligible_attackers`] — untapped creatures that
    /// pass restrictions. Human picks any subset (including empty).
    PickAttackers {
        state: StateView,
        player: PlayerId,
        eligible: Vec<InstanceId>,
    },
    /// Combat-step blocker declaration. `attackers` is the set of
    /// already-declared attackers; `eligible_blockers` is the set of
    /// defender creatures that can block. Human picks any number of
    /// `(blocker, attacker)` pairs; engine re-validates each.
    PickBlocks {
        state: StateView,
        defender: PlayerId,
        attackers: Vec<InstanceId>,
        eligible_blockers: Vec<InstanceId>,
    },
    /// Lua-handler `game.choose_card` (also hand-payment, target picks,
    /// recur picks). `pool` is the iid set the human may choose from.
    /// `host` (if any) is the card the choice is being made FOR (e.g.,
    /// the card being played, for hand-payment). `optional=true` means
    /// the human may return `null` to skip.
    ChooseCard {
        state: StateView,
        asker: PlayerId,
        pool: Vec<InstanceId>,
        host: Option<InstanceId>,
        optional: bool,
        prompt: String,
    },
    /// Lua-handler `game.confirm` (e.g., "may" abilities).
    Confirm {
        state: StateView,
        asker: PlayerId,
        prompt: String,
    },
    /// Lua-handler `game.choose_player`. `candidates` is the set the
    /// human may pick from (already filtered by `exclude`).
    ChoosePlayer {
        state: StateView,
        asker: PlayerId,
        candidates: Vec<PlayerId>,
        optional: bool,
        prompt: String,
    },
    /// Lua-handler `game.choose_int` (X-cost values, variable damage,
    /// etc.).
    ChooseInt {
        state: StateView,
        asker: PlayerId,
        min: i32,
        max: i32,
        prompt: String,
    },
    /// Game ended. No more actions accepted after this. Carries the
    /// winner so the frontend can render the result.
    GameOver {
        state: StateView,
        winner: Option<PlayerId>,
    },
}

/// Frontend's response to a [`HumanPrompt`]. Variants must match the
/// shape of the prompt that's currently outstanding — mismatched
/// responses panic. The frontend is trusted to send the right thing
/// for the prompt it last received.
///
/// Wire format: internally-tagged JSON with `kind: "..."` plus named
/// fields. Tuple variants don't survive `#[serde(tag)]`, so payloads
/// use named fields throughout.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum HumanAction {
    /// Decline to play a card this iteration of Pattern B (ends the
    /// active player's main phase).
    Pass,
    /// Play the named card from hand. Engine will then build pattern-B
    /// choices and resolve via the normal path.
    PlayCard { iid: InstanceId },
    /// Activate an ability on a board card. `x` is the human's chosen
    /// X value for X-cost activations (None for non-X abilities).
    Activate {
        iid: InstanceId,
        ability_index: usize,
        x: Option<i32>,
    },
    /// Attack with this exact subset of eligible creatures. Empty
    /// vec = no attack this turn.
    Attackers { iids: Vec<InstanceId> },
    /// Block assignments: each pair is `(blocker_iid, attacker_iid)`.
    /// Empty vec = no blocks this turn.
    Blocks { pairs: Vec<(InstanceId, InstanceId)> },
    /// Response to a [`HumanPrompt::ChooseCard`]. `iid=None` is only
    /// valid when the prompt was `optional`.
    ChoiceCard { iid: Option<InstanceId> },
    /// Response to a [`HumanPrompt::Confirm`].
    ChoiceConfirm { yes: bool },
    /// Response to a [`HumanPrompt::ChoosePlayer`]. `player=None` is
    /// only valid when the prompt was `optional`.
    ChoicePlayer { player: Option<PlayerId> },
    /// Response to a [`HumanPrompt::ChooseInt`]. Must be in
    /// `[min, max]` (engine clamps if not).
    ChoiceInt { value: i32 },
}

/// The shared object that lives in `AiKind::Human(Arc<...>)`. Engine
/// thread holds it via the AiKind; frontend holds the other ends of
/// the same channels.
///
/// Channel directions, from the engine's perspective:
/// - `prompt_tx` — engine → frontend ("here's what I need decided")
/// - `action_rx` — frontend → engine ("here's the decision")
///
/// Both channels are `std::sync::mpsc` so they're sync, blocking, no
/// async runtime dependency. The `Mutex` on `action_rx` exists because
/// `mpsc::Receiver` isn't `Sync` on its own and `AiKind` needs `Sync`
/// to live behind `Arc` shared with the frontend thread.
#[derive(Debug)]
pub struct HumanInterface {
    prompt_tx: Sender<HumanPrompt>,
    action_rx: Mutex<Receiver<HumanAction>>,
}

impl HumanInterface {
    /// Build a connected pair. The returned `(interface, prompt_rx,
    /// action_tx)` triple is wired so that wrapping `interface` in
    /// `Arc::new` and handing `prompt_rx + action_tx` to the frontend
    /// gives you a complete bridge.
    pub fn new() -> (Self, Receiver<HumanPrompt>, Sender<HumanAction>) {
        let (prompt_tx, prompt_rx) = std::sync::mpsc::channel();
        let (action_tx, action_rx) = std::sync::mpsc::channel();
        let interface = Self {
            prompt_tx,
            action_rx: Mutex::new(action_rx),
        };
        (interface, prompt_rx, action_tx)
    }

    /// Send a prompt to the frontend and block until an action returns.
    /// Caller is responsible for matching prompt variant to action
    /// variant. Panics if either channel is closed (the frontend died
    /// mid-game) — recovery is the caller's problem, not the engine's.
    fn round_trip(&self, prompt: HumanPrompt) -> HumanAction {
        self.prompt_tx
            .send(prompt)
            .expect("HumanInterface: frontend dropped prompt channel mid-game");
        self.action_rx
            .lock()
            .expect("HumanInterface: action_rx mutex poisoned")
            .recv()
            .expect("HumanInterface: frontend dropped action channel mid-game")
    }

    /// Main-phase decision. Returns what the human chose: pass, play
    /// a card, or activate an ability. The caller (Pattern B in
    /// `run_game_continue`) handles each branch.
    pub fn main_phase_choice(
        &self,
        game_state: &GameState,
        player: PlayerId,
        candidates: Vec<InstanceId>,
        kind_filter: PickKindFilter,
        activations: Vec<ActivationOption>,
    ) -> MainPhaseChoice {
        let prompt = HumanPrompt::PickCard {
            state: build_state_view(game_state, player),
            player,
            candidates,
            kind_filter,
            activations,
        };
        match self.round_trip(prompt) {
            HumanAction::Pass => MainPhaseChoice::Pass,
            HumanAction::PlayCard { iid } => MainPhaseChoice::Play(iid),
            HumanAction::Activate { iid, ability_index, x } => {
                MainPhaseChoice::Activate { iid, ability_index, x }
            }
            other => panic!("main_phase_choice: unexpected action {other:?}"),
        }
    }

    /// Combat-step attacker pick. `eligible` is the full set the human
    /// is allowed to choose from; the returned subset must be a subset
    /// of `eligible` (engine re-validates each via `declare_attacker`
    /// so an out-of-set iid is silently dropped — not a panic).
    pub fn pick_attackers(
        &self,
        game_state: &GameState,
        player: PlayerId,
        eligible: Vec<InstanceId>,
    ) -> Vec<InstanceId> {
        let prompt = HumanPrompt::PickAttackers {
            state: build_state_view(game_state, player),
            player,
            eligible,
        };
        match self.round_trip(prompt) {
            HumanAction::Attackers { iids } => iids,
            other => panic!("pick_attackers: expected Attackers, got {other:?}"),
        }
    }

    /// Combat-step block pick. The defender chooses which of its
    /// eligible blockers oppose which attackers. Pairs that fail
    /// `declare_blocker` validation are silently dropped by the
    /// engine — the human can't sneak through an illegal block.
    pub fn pick_blocks(
        &self,
        game_state: &GameState,
        defender: PlayerId,
        attackers: Vec<InstanceId>,
        eligible_blockers: Vec<InstanceId>,
    ) -> Vec<(InstanceId, InstanceId)> {
        let prompt = HumanPrompt::PickBlocks {
            state: build_state_view(game_state, defender),
            defender,
            attackers,
            eligible_blockers,
        };
        match self.round_trip(prompt) {
            HumanAction::Blocks { pairs } => pairs,
            other => panic!("pick_blocks: expected Blocks, got {other:?}"),
        }
    }

    /// Send a terminal `GameOver` notification to the frontend. Does
    /// not block — the frontend may or may not be listening; either way
    /// the engine is done.
    pub fn notify_game_over(&self, game_state: &GameState, viewer: PlayerId) {
        let _ = self.prompt_tx.send(HumanPrompt::GameOver {
            state: build_state_view(game_state, viewer),
            winner: game_state.winner,
        });
    }

    /// Lua `game.choose_card` (also hand-payment, target picks).
    pub fn choose_card(
        &self,
        game_state: &GameState,
        asker: PlayerId,
        pool: Vec<InstanceId>,
        host: Option<InstanceId>,
        optional: bool,
        prompt: String,
    ) -> Option<InstanceId> {
        let p = HumanPrompt::ChooseCard {
            state: build_state_view(game_state, asker),
            asker,
            pool,
            host,
            optional,
            prompt,
        };
        match self.round_trip(p) {
            HumanAction::ChoiceCard { iid } => iid,
            other => panic!("choose_card: expected ChoiceCard, got {other:?}"),
        }
    }

    /// Lua `game.confirm`.
    pub fn confirm(&self, game_state: &GameState, asker: PlayerId, prompt: String) -> bool {
        let p = HumanPrompt::Confirm {
            state: build_state_view(game_state, asker),
            asker,
            prompt,
        };
        match self.round_trip(p) {
            HumanAction::ChoiceConfirm { yes } => yes,
            other => panic!("confirm: expected ChoiceConfirm, got {other:?}"),
        }
    }

    /// Lua `game.choose_player`.
    pub fn choose_player(
        &self,
        game_state: &GameState,
        asker: PlayerId,
        candidates: Vec<PlayerId>,
        optional: bool,
        prompt: String,
    ) -> Option<PlayerId> {
        let p = HumanPrompt::ChoosePlayer {
            state: build_state_view(game_state, asker),
            asker,
            candidates,
            optional,
            prompt,
        };
        match self.round_trip(p) {
            HumanAction::ChoicePlayer { player } => player,
            other => panic!("choose_player: expected ChoicePlayer, got {other:?}"),
        }
    }

    /// Lua `game.choose_int` (X-costs, variable damage).
    pub fn choose_int(
        &self,
        game_state: &GameState,
        asker: PlayerId,
        min: i32,
        max: i32,
        prompt: String,
    ) -> i32 {
        let p = HumanPrompt::ChooseInt {
            state: build_state_view(game_state, asker),
            asker,
            min,
            max,
            prompt,
        };
        match self.round_trip(p) {
            HumanAction::ChoiceInt { value } => value.clamp(min, max),
            other => panic!("choose_int: expected ChoiceInt, got {other:?}"),
        }
    }
}

/// Wraps any `ChoiceOracle` (typically the sim's `RandomOracle`) and
/// routes calls to a [`HumanInterface`] when the asker matches the
/// configured human side. Calls from the AI side (or any side without
/// a configured human) fall through to `inner`.
///
/// `ChoosePlayerRequest` and `ChooseIntRequest` don't carry an `asker`
/// field. For those, we fall back to `state.active_player` as the
/// implicit asker — correct for the common cases (handlers fired on
/// the controller's own turn; X-cost picks during cast).
pub struct HumanAwareOracle<O: ChoiceOracle> {
    inner: O,
    human: Option<(PlayerId, Arc<HumanInterface>)>,
}

impl<O: ChoiceOracle> HumanAwareOracle<O> {
    pub fn new(inner: O, human: Option<(PlayerId, Arc<HumanInterface>)>) -> Self {
        Self { inner, human }
    }

    fn human_for(&self, asker: PlayerId) -> Option<&HumanInterface> {
        self.human
            .as_ref()
            .filter(|(side, _)| *side == asker)
            .map(|(_, iface)| iface.as_ref())
    }
}

impl<O: ChoiceOracle> ChoiceOracle for HumanAwareOracle<O> {
    fn choose_card(
        &mut self,
        state: &GameState,
        req: ChooseCardRequest,
    ) -> Option<InstanceId> {
        if let Some(asker) = req.asker {
            if let Some(iface) = self.human_for(asker) {
                return iface.choose_card(
                    state,
                    asker,
                    req.pool,
                    req.host,
                    req.optional,
                    req.prompt,
                );
            }
        }
        self.inner.choose_card(state, req)
    }

    fn confirm(&mut self, state: &GameState, asker: PlayerId, prompt: &str) -> bool {
        if let Some(iface) = self.human_for(asker) {
            return iface.confirm(state, asker, prompt.to_string());
        }
        self.inner.confirm(state, asker, prompt)
    }

    fn choose_player(
        &mut self,
        state: &GameState,
        req: ChoosePlayerRequest,
    ) -> Option<PlayerId> {
        // Asker isn't carried on the request; use active_player as the
        // implicit asker (correct for handlers triggered on the
        // controller's turn).
        let asker = state.active_player;
        if let Some(iface) = self.human_for(asker) {
            let exclude = req.exclude.clone();
            let candidates: Vec<PlayerId> = [PlayerId::A, PlayerId::B]
                .into_iter()
                .filter(|p| !exclude.contains(p))
                .collect();
            return iface.choose_player(state, asker, candidates, req.optional, req.prompt);
        }
        self.inner.choose_player(state, req)
    }

    fn choose_int(&mut self, state: &GameState, req: ChooseIntRequest) -> i32 {
        let asker = state.active_player;
        if let Some(iface) = self.human_for(asker) {
            return iface.choose_int(state, asker, req.min, req.max, req.prompt);
        }
        self.inner.choose_int(state, req)
    }

    fn set_next_intent(&mut self, intent: Option<TargetIntent>) {
        // Intent hints don't reach the human (handler-author scoring
        // doesn't apply to a human picker). Still pass-through to the
        // inner oracle so AI-side calls keep their scoring.
        self.inner.set_next_intent(intent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use crate::card::{CardRegistry, CardType};
    use crate::game::{GameState, Journal};

    /// Run a full game with `AiKind::Human` on side A. The "human" is a
    /// helper thread that runs a passive script: pass on every card
    /// pick, no attackers, no blocks. The heuristic-driven B side does
    /// whatever it wants and should eventually win (or the game ends
    /// some other way). Proves the channel wiring works end-to-end —
    /// engine sends prompts, script answers, engine progresses, game
    /// terminates.
    #[test]
    fn passive_human_loses_to_heuristic_without_deadlock() {
        use crate::sim::run::run_game_continue;
        use crate::sim::AiKind;

        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| matches!(c.kind, CardType::Creature) && c.handlers.is_empty())
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let mut state = GameState::new(deck_a, deck_b);
        state.replay_journal = Some(Journal::new());

        let (iface, prompt_rx, action_tx) = HumanInterface::new();
        let iface = Arc::new(iface);

        // Helper thread: dumb passive script. Pass everything.
        let script = thread::spawn(move || {
            let mut prompts_seen = 0u32;
            while let Ok(prompt) = prompt_rx.recv() {
                prompts_seen += 1;
                let action = match prompt {
                    HumanPrompt::PickCard { .. } => HumanAction::Pass,
                    HumanPrompt::PickAttackers { .. } => HumanAction::Attackers { iids: Vec::new() },
                    HumanPrompt::PickBlocks { .. } => HumanAction::Blocks { pairs: Vec::new() },
                    HumanPrompt::ChooseCard { optional, pool, .. } => HumanAction::ChoiceCard {
                        iid: if optional { None } else { pool.first().cloned() },
                    },
                    HumanPrompt::Confirm { .. } => HumanAction::ChoiceConfirm { yes: false },
                    HumanPrompt::ChoosePlayer { optional, candidates, .. } => HumanAction::ChoicePlayer {
                        player: if optional { None } else { candidates.first().copied() },
                    },
                    HumanPrompt::ChooseInt { min, .. } => HumanAction::ChoiceInt { value: min },
                    HumanPrompt::GameOver { .. } => break,
                };
                if action_tx.send(action).is_err() {
                    break;
                }
            }
            prompts_seen
        });

        let ais = [AiKind::Human(iface.clone()), AiKind::Heuristic];
        let mut rng = StdRng::seed_from_u64(0xCAFE);
        let mut log: Vec<String> = Vec::new();
        let stats = run_game_continue(&mut state, &mut rng, &mut log, registry.lua(), &ais);

        // Engine returned. Drop everything that holds prompt_tx (ais
        // owns an Arc<HumanInterface> which holds prompt_tx) so the
        // script thread's recv() sees disconnection and exits.
        drop(ais);
        drop(iface);
        drop(state);
        let prompts_seen = script.join().expect("script thread panicked");

        assert!(prompts_seen > 0, "script thread never saw a prompt");
        assert!(stats.turns > 0, "game recorded zero turns");
    }
}
