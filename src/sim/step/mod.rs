//! Engine state-machine entry point. See `STATE_MACHINE.md` for the
//! design + phased plan; this module is the S1 scaffold (types only,
//! no phase logic yet).
//!
//! Once S2-S13 land, [`StepEngine`] is the only way to drive a game.
//! [`crate::sim::run::run_game_continue`] becomes a thin wrapper that
//! constructs a `StepEngine` and loops `step()` until `Done`. WASM
//! FFI uses `step()` directly so each FFI call advances one decision
//! distance.

#![allow(dead_code)] // S1 scaffold; fields used as S2+ fills phase logic.

use std::collections::{BTreeMap, BTreeSet};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::card::CardRegistry;
use crate::choice::{RandomOracle, RecordingOracle};
use crate::game::{EventContext, GameState, InstanceId, Phase, PlayerId};
use crate::sim::human::{HumanAction, HumanPrompt};
use crate::sim::stats::GameStats;
use crate::sim::variants::DeckVariant;
use crate::sim::AiKind;

mod combat;
mod main_phases;
#[cfg(test)]
mod tests;

/// Where the engine is in the game flow. One variant per yield-able
/// decision point + the boundary states (start of turn / game over).
///
/// S1 declares only the boundary variants. S2 fills in the AI-driven
/// non-yielding flow (TurnSetup → Main1 → Combat → EndTurn). S4 / S5
/// add the Human-yielding variants (`PatternBPick`, `DeclareAttackers`,
/// `DeclareBlockers`). S7-S10 add `ChoiceOracle` and activation
/// sub-cursors. Subject to refinement once we actually port
/// `run_game_continue`'s loop state into struct fields.
#[derive(Debug, Clone)]
pub enum EngineCursor {
    /// Initial state — engine constructed but no step() called yet.
    /// Transitions to TurnSetup on first step().
    StartTurn,
    /// Advance phases until Phase::Main1, then enter PatternBPick.
    /// Empty-deck-out checks happen here (`state.next_phase()` can
    /// set `state.winner` if the active player can't draw).
    TurnSetup,
    /// Main-phase card-pick decision. `played_creature` tracks
    /// whether the active player has already cast a creature this
    /// turn (Pattern B's one-creature-per-turn cap; non-creatures
    /// stay free to cast multiple per turn).
    PatternBPick { played_creature: bool },
    /// S7: a human committed to playing `picked`, but
    /// `build_pattern_b_choices` (or a downstream oracle call inside
    /// `play_card`) needs further answers. `history` is the
    /// `ChoiceCard` / `ChoiceConfirm` / `ChoicePlayer` / `ChoiceInt`
    /// responses the human has supplied so far for this resolve; the
    /// engine seeds `HumanReplayOracle` with these on every retry
    /// until the resolve completes.
    PatternBResolving {
        picked: InstanceId,
        history: Vec<crate::choice::ScriptedAnswer>,
        played_creature_before: bool,
    },
    /// S9: pre-combat activation pass. Runs after Pattern B exits,
    /// before `DeclareAttackers`. AI-active player auto-fires each
    /// eligible activated ability on non-creature board cards
    /// (matching `run_activation_pass(non_creatures_only = true)`'s
    /// behavior in `run_game_continue`). Human-active player skips
    /// this cursor — human activations happen inline in `PatternBPick`
    /// via the `HumanAction::Activate { … }` response variant.
    PreCombatActivations,
    /// Combat: choose attackers. AI dispatch uses `select_attackers`.
    DeclareAttackers,
    /// Attackers declared via `state.declare_attacker`; now ask the
    /// defender for blockers (or skip to EndTurn if none attacked).
    DeclareBlockers,
    /// S9: post-combat activation pass. Runs after `DeclareBlockers`
    /// (combat damage resolved) and before `EndTurn`. AI-active
    /// player auto-fires each eligible activated ability on every
    /// board card — including creatures (vigilance still-untapped
    /// after swinging draws / pumps here). Mirrors
    /// `run_activation_pass(non_creatures_only = false)`.
    PostCombatActivations,
    /// S10: human-active player's Main2 prompt loop. AI-active turns
    /// skip straight from `PostCombatActivations` to `EndTurn`; human
    /// turns route through here so the player can cast / activate /
    /// pass during their second main phase. Mirrors the Pattern B
    /// shape but inside `Phase::Main2` so sorcery-speed timing is
    /// correct. `played_creature` tracks Main2's own one-creature cap
    /// (per `run_game_continue`'s `m2_played_creature` — fresh from
    /// Main1's cap).
    Main2Pick { played_creature: bool },
    /// S10: human committed to a Main2 play but the resolve needs
    /// more `ChoiceCard / Confirm / Player / Int` answers. Same
    /// replay-history protocol as `PatternBResolving`; on completion
    /// the cursor advances back to `Main2Pick` so the human can
    /// chain more plays (Pattern B's one-creature-per-turn cap still
    /// applies — tracked via `played_creature`).
    Main2Resolving {
        picked: InstanceId,
        history: Vec<crate::choice::ScriptedAnswer>,
        played_creature: bool,
    },
    /// All combat resolved; advance phases past End so the next turn
    /// can start. On wrap, transitions to StartTurn.
    EndTurn,
    /// Game ended; subsequent step() calls return `Done(stats)` with
    /// the final stats snapshot.
    GameOver,
}

/// What a single step() call did. Caller loops as long as `Continue`
/// is returned; surfaces `NeedHuman` to JS / UI / test code; stops
/// looping on `Done`.
#[derive(Debug)]
pub enum StepResult {
    /// Engine advanced; the cursor was mutated. Call step() again
    /// immediately with `pending = None`.
    Continue,
    /// The next decision requires a HumanAction. Caller resumes by
    /// passing the action as `pending` to the next step() call.
    /// Boxed because `HumanPrompt` is the largest variant by a wide
    /// margin and a non-boxed copy bloats every `StepResult` return.
    NeedHuman(Box<HumanPrompt>),
    /// Game ended. Stats snapshot included; further step() calls
    /// stay in `Done`.
    Done(Box<GameStats>),
}

/// Owned engine state for step-mode execution. Constructed once via
/// [`StepEngine::new`], driven via repeated [`StepEngine::step`]
/// calls. Owns the `mlua::Lua` (`registry` field), so the engine is
/// `!Send`; each browser tab / FFI session has its own.
pub struct StepEngine {
    pub state: GameState,
    pub cursor: EngineCursor,
    pub ais: [AiKind; 2],
    /// S12: registry is shared via `Arc` so callers that already own
    /// (or only borrow) a `CardRegistry` — MCTS / UCT rollouts hand-roll
    /// a per-rollout engine, the wasm session shares with the JS-facing
    /// FFI layer, the EA loop reuses one registry across many games —
    /// can hand a reference into the engine without giving up ownership
    /// or reloading from disk.
    pub registry: std::sync::Arc<CardRegistry>,
    pub rng: StdRng,
    pub stats: GameStats,
    pub log: Vec<String>,
    /// S7: the engine's oracle is `RecordingOracle<HumanReplayOracle<…>>`
    /// rather than `…HumanAwareOracle<…>`. The replay layer captures
    /// `choose_*` requests as `Err(ChoicePending)` whenever the human's
    /// replay queue is exhausted — the engine lifts that into a
    /// `NeedHuman` yield instead of blocking on a channel.
    pub oracle: RecordingOracle<crate::sim::human::HumanReplayOracle<RandomOracle<StdRng>>>,
}

impl StepEngine {
    /// Build a fresh engine in the StartTurn cursor. `seed` derives
    /// the engine's RNG sequence; this matches the `run_game` /
    /// `run_game_continue` determinism contract — same seed + same
    /// inputs → byte-identical step sequence.
    pub fn new(
        state: GameState,
        ais: [AiKind; 2],
        registry: impl Into<std::sync::Arc<CardRegistry>>,
        seed: u64,
    ) -> Self {
        let registry = registry.into();
        let mut rng = StdRng::seed_from_u64(seed);
        // Burn one rng tick on the oracle seed to match
        // `run_game_continue`'s rng-consumption order (so a same-seed
        // game produces the same trajectory through both paths).
        let oracle_seed: u64 = rng.gen();
        let human_side: Option<PlayerId> = ais
            .iter()
            .enumerate()
            .find_map(|(idx, ai)| match ai {
                AiKind::Human(_) => Some(if idx == 0 { PlayerId::A } else { PlayerId::B }),
                _ => None,
            });
        let oracle = RecordingOracle::new(crate::sim::human::HumanReplayOracle::new(
            RandomOracle::new(StdRng::seed_from_u64(oracle_seed)),
            human_side,
        ));

        Self {
            state,
            cursor: EngineCursor::StartTurn,
            ais,
            registry,
            rng,
            stats: fresh_game_stats(),
            log: Vec::new(),
            oracle,
        }
    }
}

/// Lift a `ChoicePending` from the oracle into a `HumanPrompt` the
/// engine can yield. The viewer is the asker for `Card` requests
/// (which carry their own `asker`); for `Confirm` it's the named
/// asker; for `Player` / `Int` we use the active player (the same
/// convention `HumanAwareOracle` uses when those requests don't carry
/// an asker field).
fn pending_to_prompt(state: &GameState, pending: crate::choice::ChoicePending) -> HumanPrompt {
    use crate::choice::ChoicePending;
    match pending {
        ChoicePending::Card(req) => {
            let viewer = req.asker.unwrap_or(state.active_player);
            HumanPrompt::ChooseCard {
                state: crate::sim::snapshot::build_state_view(state, viewer),
                asker: viewer,
                pool: req.pool,
                host: req.host,
                optional: req.optional,
                prompt: req.prompt,
            }
        }
        ChoicePending::Confirm { asker, prompt } => HumanPrompt::Confirm {
            state: crate::sim::snapshot::build_state_view(state, asker),
            asker,
            prompt,
        },
        ChoicePending::Player(req) => {
            let asker = state.active_player;
            let candidates: Vec<PlayerId> = [PlayerId::A, PlayerId::B]
                .into_iter()
                .filter(|p| !req.exclude.contains(p))
                .collect();
            HumanPrompt::ChoosePlayer {
                state: crate::sim::snapshot::build_state_view(state, asker),
                asker,
                candidates,
                optional: req.optional,
                prompt: req.prompt,
            }
        }
        ChoicePending::Int(req) => {
            let asker = state.active_player;
            HumanPrompt::ChooseInt {
                state: crate::sim::snapshot::build_state_view(state, asker),
                asker,
                min: req.min,
                max: req.max,
                prompt: req.prompt,
            }
        }
    }
}

/// Same initial `GameStats` layout that `run_game_continue` uses.
/// Extracted here so the step engine and the legacy runner start
/// from byte-identical stats and S3's parity check works.
fn fresh_game_stats() -> GameStats {
    GameStats {
        turns: 0,
        winner: PlayerId::A,
        variant_a: DeckVariant::Ra,
        variant_b: DeckVariant::Rb,
        token_a: String::new(),
        token_b: String::new(),
        game_index: 0,
        deck_a_ids: BTreeSet::new(),
        deck_b_ids: BTreeSet::new(),
        a_played_card_ids: BTreeSet::new(),
        b_played_card_ids: BTreeSet::new(),
        card_play_turns: BTreeMap::new(),
        card_play_turn_events: Vec::new(),
        card_sacrificed_count: BTreeMap::new(),
        card_discarded_count: BTreeMap::new(),
        a_played: 0,
        b_played: 0,
        a_attacks: 0,
        b_attacks: 0,
        a_deaths: 0,
        b_deaths: 0,
        a_milled_to_exile: 0,
        b_milled_to_exile: 0,
        a_final_board: 0,
        b_final_board: 0,
        a_final_gy: 0,
        b_final_gy: 0,
        a_preview_attempts: 0,
        b_preview_attempts: 0,
        a_preview_rollbacks: 0,
        b_preview_rollbacks: 0,
        a_preview_journal_size_total: 0,
        b_preview_journal_size_total: 0,
        replay_journal_entries: 0,
        event_fires: BTreeMap::new(),
        action_counts: BTreeMap::new(),
    }
}

impl StepEngine {
    /// Advance the engine one transition. Returns `Continue` to keep
    /// driving via `step(None)`, `NeedHuman` to surface a prompt back
    /// to the caller, or `Done` when the game has ended.
    ///
    /// S2 scope: vanilla decks (no Lua handlers, no activated
    /// abilities, no X-cost), AI-only dispatch (Heuristic / MCTS /
    /// UCT). Human dispatch arrives in S4-S5; activations + Lua in
    /// S7-S10; the surrounding edge cases (suicide rollback,
    /// response windows) in S11.
    pub fn step(&mut self, pending: Option<HumanAction>) -> StepResult {
        // Game-over short circuit: any cursor with state.winner set
        // collapses into GameOver. This keeps the per-cursor logic
        // below from needing to repeat the check.
        if self.state.winner.is_some() && !matches!(self.cursor, EngineCursor::GameOver) {
            self.finalize_stats();
            self.cursor = EngineCursor::GameOver;
            return StepResult::Done(Box::new(self.stats.clone()));
        }

        match self.cursor.clone() {
            EngineCursor::StartTurn => {
                self.cursor = EngineCursor::TurnSetup;
                StepResult::Continue
            }
            EngineCursor::TurnSetup => {
                // Advance to Main1; `next_phase` runs Untap / Draw /
                // upkeep events, which can set `state.winner` on
                // deck-out. The game-over check at the top of the
                // next `step()` call catches that. Use a fresh oracle
                // per advance to match run_game_continue's RNG
                // consumption order.
                while self.state.phase != Phase::Main1 && self.state.winner.is_none() {
                    let mut oracle =
                        RandomOracle::new(StdRng::seed_from_u64(self.rng.gen()));
                    self.state.next_phase(Some(&mut EventContext::new(
                        self.registry.lua(),
                        &mut oracle,
                    )));
                }
                self.cursor = EngineCursor::PatternBPick {
                    played_creature: false,
                };
                StepResult::Continue
            }
            EngineCursor::PatternBPick { played_creature } => {
                self.step_pattern_b_pick(played_creature, pending)
            }
            EngineCursor::PatternBResolving {
                picked,
                history,
                played_creature_before,
            } => self.step_pattern_b_resolve(picked, history, played_creature_before, pending),
            EngineCursor::PreCombatActivations => self.step_activation_pass(true),
            EngineCursor::DeclareAttackers => self.step_declare_attackers(pending),
            EngineCursor::DeclareBlockers => self.step_declare_blockers(pending),
            EngineCursor::PostCombatActivations => self.step_activation_pass(false),
            EngineCursor::Main2Pick { played_creature } => {
                self.step_main2_pick(played_creature, pending)
            }
            EngineCursor::Main2Resolving {
                picked,
                history,
                played_creature,
            } => self.step_main2_resolve(picked, history, played_creature, pending),
            EngineCursor::EndTurn => {
                // Advance phases until the turn ticks (End → next
                // Untap on the other side). `state.winner` may be set
                // along the way; caught next iteration. Fresh oracle
                // per advance matches run_game_continue's RNG order.
                let starting_turn = self.state.turn;
                while self.state.turn == starting_turn && self.state.winner.is_none() {
                    let mut oracle =
                        RandomOracle::new(StdRng::seed_from_u64(self.rng.gen()));
                    self.state.next_phase(Some(&mut EventContext::new(
                        self.registry.lua(),
                        &mut oracle,
                    )));
                }
                self.cursor = EngineCursor::StartTurn;
                StepResult::Continue
            }
            EngineCursor::GameOver => StepResult::Done(Box::new(self.stats.clone())),
        }
    }

    // Pattern B / Main2 / combat / activation handlers moved to the
    // `main_phases` and `combat` submodules.

    /// Drive `step(None)` to completion. Equivalent to the legacy
    /// `run_game_continue` for AI-only games. Will be migrated to
    /// be the only entry point in S12.
    pub fn run_to_end(&mut self) -> GameStats {
        loop {
            match self.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(prompt) => panic!(
                    "run_to_end: engine asked for human input it can't provide ({prompt:?})"
                ),
                StepResult::Done(stats) => return *stats,
            }
        }
    }

    /// Populate `stats.turns` / final-board / final-graveyard from
    /// the current state. Called once on transition into GameOver.
    fn finalize_stats(&mut self) {
        self.stats.turns = self.state.turn;
        self.stats.winner = self.state.winner.unwrap_or(PlayerId::A);
        self.stats.a_final_board = self.state.a.board.len() as u32;
        self.stats.b_final_board = self.state.b.board.len() as u32;
        self.stats.a_final_gy = self.state.a.graveyard.len() as u32;
        self.stats.b_final_gy = self.state.b.graveyard.len() as u32;
        self.stats.event_fires = self.state.event_fires.clone();
        self.stats.action_counts = self.state.action_counts.clone();
    }
}
