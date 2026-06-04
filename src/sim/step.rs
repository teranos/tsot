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

use crate::card::{CardRegistry, CardType};
use crate::choice::{RandomOracle, RecordingOracle};
use crate::game::{EventContext, GameState, InstanceId, Phase, PlayerId};
use crate::sim::ai::{
    eligible_attackers, eligible_blockers, pick_blocks, pick_random_playable_in_hand,
    select_attackers, PickKindFilter,
};
use crate::sim::human::{HumanAction, HumanPrompt};
use crate::sim::run::{build_pattern_b_choices, BuildChoiceResult};
use crate::sim::stats::{
    bump_attacks, bump_milled, bump_played, bump_preview_attempt, bump_preview_rollback, GameStats,
};
use crate::sim::variants::DeckVariant;
use crate::sim::AiKind;

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
    pub registry: CardRegistry,
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
        registry: CardRegistry,
        seed: u64,
    ) -> Self {
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

    /// Pattern B: pick a card to play (or pass into combat).
    /// Vanilla AI scope unchanged; the Human arm yields a
    /// `NeedHuman(PickCard{…})` on `pending=None` and resumes from
    /// the human's `HumanAction::Pass` / `PlayCard` on the next call.
    /// `Activate` (S9) currently re-prompts.
    fn step_pattern_b_pick(
        &mut self,
        played_creature: bool,
        pending: Option<HumanAction>,
    ) -> StepResult {
        let active = self.state.active_player;
        let kind_filter = if played_creature {
            PickKindFilter::NonCreatureOnly
        } else {
            PickKindFilter::Any
        };

        let pick = match &self.ais[active.index()] {
            AiKind::Heuristic => pick_random_playable_in_hand(
                &self.state,
                active,
                &mut self.rng,
                kind_filter,
            ),
            AiKind::Mcts(cfg) => {
                let cfg = cfg.clone();
                crate::sim::mcts::pick_play(
                    &mut self.state,
                    active,
                    kind_filter,
                    &cfg,
                    self.registry.lua(),
                )
            }
            AiKind::Uct(cfg) => {
                let cfg = cfg.clone();
                crate::sim::uct::pick_play_uct(
                    &mut self.state,
                    active,
                    kind_filter,
                    &cfg,
                    self.registry.lua(),
                )
            }
            AiKind::Human(_) => match pending {
                None => {
                    let candidates = crate::sim::ai::enumerate_playable_in_hand(
                        &self.state,
                        active,
                        kind_filter,
                    );
                    let activations =
                        crate::sim::run::enumerate_human_activations(&self.state, active);
                    let prompt = HumanPrompt::PickCard {
                        state: crate::sim::snapshot::build_state_view(&self.state, active),
                        player: active,
                        candidates,
                        kind_filter,
                        activations,
                    };
                    return StepResult::NeedHuman(Box::new(prompt));
                }
                Some(HumanAction::Pass) => None,
                Some(HumanAction::PlayCard { iid }) => Some(iid),
                Some(HumanAction::Activate { .. }) => {
                    // S9: human activations through Pattern B. For now,
                    // ignore the action and re-prompt so the frontend
                    // can resend Pass / PlayCard.
                    let candidates = crate::sim::ai::enumerate_playable_in_hand(
                        &self.state,
                        active,
                        kind_filter,
                    );
                    let activations =
                        crate::sim::run::enumerate_human_activations(&self.state, active);
                    let prompt = HumanPrompt::PickCard {
                        state: crate::sim::snapshot::build_state_view(&self.state, active),
                        player: active,
                        candidates,
                        kind_filter,
                        activations,
                    };
                    return StepResult::NeedHuman(Box::new(prompt));
                }
                Some(other) => panic!(
                    "PatternBPick: expected Pass / PlayCard / Activate response, got {other:?}"
                ),
            },
        };

        let Some(picked) = pick else {
            // No more plays this turn; advance into the pre-combat
            // activation pass (S9), then combat.
            self.cursor = EngineCursor::PreCombatActivations;
            return StepResult::Continue;
        };

        // Resolve the pick: build choices + play the card. This still
        // delegates to `build_pattern_b_choices` + `play_card` — those
        // remain monolithic; we just call them per pick from inside the
        // step state machine.
        let picked_is_creature = self
            .state
            .card_pool
            .get(&picked)
            .map(|c| c.card.kind == CardType::Creature)
            .unwrap_or(false);
        let kind = self
            .state
            .card_pool
            .get(&picked)
            .map(|c| c.card.kind)
            .unwrap_or(CardType::Unspecified);

        // Order matches run_game_continue: build_pattern_b_choices runs
        // with journal=None so `rig_creature_free_haste`'s cost-clear is
        // a permanent mutation (rig sits OUTSIDE the preview-rollback
        // envelope by design). Only THEN open the preview journal for
        // play_card's mutations.
        let build_result = build_pattern_b_choices(
            &mut self.state,
            active,
            &picked,
            &mut self.oracle,
            matches!(self.ais[active.index()], AiKind::Human(_)),
        );
        let choices = match build_result {
            BuildChoiceResult::Choices(c) => c,
            BuildChoiceResult::UnaffordableX { picked_is_creature: pic } => {
                // Same loop-advance as run.rs: if it was a creature,
                // mark played_creature so we don't re-pick it; else
                // bail out of Pattern B and head into combat.
                self.cursor = if pic {
                    EngineCursor::PatternBPick {
                        played_creature: true,
                    }
                } else {
                    EngineCursor::PreCombatActivations
                };
                return StepResult::Continue;
            }
            BuildChoiceResult::Pending(p) => {
                // S7: oracle needs the human's answer. Snapshot what
                // we'd need to retry the resolve, transition into the
                // resolving cursor, and yield the prompt. The next
                // step() call will land in `step_pattern_b_resolve`.
                self.cursor = EngineCursor::PatternBResolving {
                    picked: picked.clone(),
                    history: Vec::new(),
                    played_creature_before: played_creature,
                };
                let prompt = pending_to_prompt(&self.state, p);
                return StepResult::NeedHuman(Box::new(prompt));
            }
        };

        // Sacrifice telemetry (matches run.rs).
        for sac_iid in &choices.sacrifice_ids {
            if let Some(card_id) = self
                .state
                .card_pool
                .get(sac_iid)
                .map(|c| c.card.id.clone())
            {
                *self
                    .stats
                    .card_sacrificed_count
                    .entry(card_id)
                    .or_insert(0) += 1;
            }
        }
        self.oracle.clear();

        // Open the per-cast preview journal so play_card's mutations
        // are captured for suicide rollback / response-window rollback.
        self.state.journal = Some(crate::game::Journal::new());

        let opponent_of_active = active.opponent();
        let preview_size_before = self
            .state
            .journal
            .as_ref()
            .map(|j| j.len())
            .unwrap_or(0) as u64;

        let result = self.state.play_card(
            active,
            &picked,
            choices,
            Some(&mut EventContext::new(self.registry.lua(), &mut self.oracle)),
        );

        let suicide = !matches!(self.ais[active.index()], AiKind::Human(_))
            && self.state.winner == Some(opponent_of_active);
        let preview_size = self
            .state
            .journal
            .as_ref()
            .map(|j| j.len())
            .unwrap_or(0) as u64;
        bump_preview_attempt(&mut self.stats, active, preview_size.max(preview_size_before));

        if result.is_ok() && !suicide {
            if let Some(mut preview) = self.state.journal.take() {
                if let Some(replay) = self.state.replay_journal.as_mut() {
                    replay.extend_from(&mut preview);
                }
            }
            bump_played(&mut self.stats, active);
            // Card-tracking telemetry (matches run.rs).
            if let Some(card_id) = self
                .state
                .card_pool
                .get(&picked)
                .map(|c| c.card.id.clone())
            {
                match active {
                    PlayerId::A => {
                        self.stats.a_played_card_ids.insert(card_id.clone());
                    }
                    PlayerId::B => {
                        self.stats.b_played_card_ids.insert(card_id.clone());
                    }
                }
                let turn_now = self.state.turn;
                self.stats
                    .card_play_turns
                    .entry(card_id.clone())
                    .and_modify(|(min_t, max_t)| {
                        if turn_now < *min_t {
                            *min_t = turn_now;
                        }
                        if turn_now > *max_t {
                            *max_t = turn_now;
                        }
                    })
                    .or_insert((turn_now, turn_now));
                self.stats
                    .card_play_turn_events
                    .push((card_id, turn_now, active));
            }
            let new_played_creature = played_creature || picked_is_creature;
            self.cursor = EngineCursor::PatternBPick {
                played_creature: new_played_creature,
            };
        } else {
            // Failure (or suicide): rollback the preview journal.
            if let Some(journal) = self.state.journal.take() {
                journal.rollback(&mut self.state);
            }
            bump_preview_rollback(&mut self.stats, active);
            if suicide {
                self.state.bump_action("preview_skip_suicide", active);
            }
            // Same advance heuristic as run.rs: creature failures
            // mark played_creature so we stop re-picking the same
            // suicidal creature; non-creature failures bail.
            if picked_is_creature {
                self.cursor = EngineCursor::PatternBPick {
                    played_creature: true,
                };
            } else {
                self.cursor = EngineCursor::PreCombatActivations;
            }
        }

        let _ = kind;
        StepResult::Continue
    }

    fn step_declare_attackers(&mut self, pending: Option<HumanAction>) -> StepResult {
        // Advance Main1 → Combat. declare_attacker rejects with
        // `WrongPhase` outside Combat; run_game_continue advances
        // here after its Pattern B loop ends. Fresh oracle per
        // advance matches run_game_continue's RNG order.
        while self.state.phase != Phase::Combat && self.state.winner.is_none() {
            let mut oracle = RandomOracle::new(StdRng::seed_from_u64(self.rng.gen()));
            self.state.next_phase(Some(&mut EventContext::new(
                self.registry.lua(),
                &mut oracle,
            )));
        }
        if self.state.winner.is_some() {
            return StepResult::Continue;
        }
        let active = self.state.active_player;
        let attackers = match &self.ais[active.index()] {
            AiKind::Heuristic | AiKind::Mcts(_) | AiKind::Uct(_) => {
                select_attackers(&self.state, active)
            }
            AiKind::Human(_) => match pending {
                None => {
                    let eligible = eligible_attackers(&self.state, active);
                    let prompt = HumanPrompt::PickAttackers {
                        state: crate::sim::snapshot::build_state_view(&self.state, active),
                        player: active,
                        eligible,
                    };
                    return StepResult::NeedHuman(Box::new(prompt));
                }
                Some(HumanAction::Attackers { iids }) => iids,
                Some(other) => panic!(
                    "DeclareAttackers: expected Attackers response, got {other:?}"
                ),
            },
        };
        let mut declared_atk_count = 0u32;
        for atk in &attackers {
            if self
                .state
                .declare_attacker(
                    atk,
                    Some(&mut EventContext::new(self.registry.lua(), &mut self.oracle)),
                )
                .is_ok()
            {
                declared_atk_count += 1;
            }
        }
        if declared_atk_count > 0 {
            self.state.confirm_attacks().unwrap();
            self.cursor = EngineCursor::DeclareBlockers;
        } else {
            // No attackers declared → skip blockers, still run the
            // post-combat activation pass (run_game_continue runs it
            // unconditionally).
            self.cursor = EngineCursor::PostCombatActivations;
        }
        // Eligibility list is consumed; let the compiler know.
        let _ = eligible_attackers(&self.state, active);
        bump_attacks(&mut self.stats, active, declared_atk_count);
        StepResult::Continue
    }

    /// S7: resume a human play that previously yielded `NeedHuman`
    /// because the oracle ran out of replay entries. Appends the
    /// freshly-supplied answer to `history`, retries the resolve from
    /// scratch with the full history pre-loaded into the oracle. If
    /// the retry yields again, we stay in this cursor.
    fn step_pattern_b_resolve(
        &mut self,
        picked: InstanceId,
        mut history: Vec<crate::choice::ScriptedAnswer>,
        played_creature_before: bool,
        pending: Option<HumanAction>,
    ) -> StepResult {
        let active = self.state.active_player;
        // Push the human's response (if any) onto the replay history.
        // None = the resume tick before we've received an action; that
        // shouldn't happen but tolerate it gracefully.
        if let Some(act) = pending {
            let ans = match act {
                HumanAction::ChoiceCard { iid } => crate::choice::ScriptedAnswer::Card(iid),
                HumanAction::ChoiceConfirm { yes } => crate::choice::ScriptedAnswer::Confirm(yes),
                HumanAction::ChoicePlayer { player } => {
                    crate::choice::ScriptedAnswer::Player(player)
                }
                HumanAction::ChoiceInt { value } => crate::choice::ScriptedAnswer::Int(value),
                other => panic!(
                    "PatternBResolving: expected Choice* response, got {other:?}"
                ),
            };
            history.push(ans);
        }

        // Pre-load the replay queue and clear the recorder (which
        // already captured the failed attempt's mock answers).
        self.oracle.clear();
        self.oracle.inner_mut().reset_replay(history.clone());

        let picked_is_creature = self
            .state
            .card_pool
            .get(&picked)
            .map(|c| c.card.kind == CardType::Creature)
            .unwrap_or(false);

        let build_result = build_pattern_b_choices(
            &mut self.state,
            active,
            &picked,
            &mut self.oracle,
            true, // active_is_human (this cursor only fires for human side)
        );
        let choices = match build_result {
            BuildChoiceResult::Choices(c) => c,
            BuildChoiceResult::UnaffordableX { .. } => {
                // Same fall-through as step_pattern_b_pick: creature
                // that turned unaffordable means we mark the cap and
                // re-enter PatternBPick; non-creature bails to combat.
                self.cursor = if picked_is_creature {
                    EngineCursor::PatternBPick {
                        played_creature: true,
                    }
                } else {
                    EngineCursor::DeclareAttackers
                };
                return StepResult::Continue;
            }
            BuildChoiceResult::Pending(p) => {
                self.cursor = EngineCursor::PatternBResolving {
                    picked,
                    history,
                    played_creature_before,
                };
                let prompt = pending_to_prompt(&self.state, p);
                return StepResult::NeedHuman(Box::new(prompt));
            }
        };

        // Sacrifice telemetry (matches step_pattern_b_pick).
        for sac_iid in &choices.sacrifice_ids {
            if let Some(card_id) = self
                .state
                .card_pool
                .get(sac_iid)
                .map(|c| c.card.id.clone())
            {
                *self
                    .stats
                    .card_sacrificed_count
                    .entry(card_id)
                    .or_insert(0) += 1;
            }
        }
        self.oracle.clear();

        // Open the per-cast preview journal so play_card mutations
        // can be rolled back (parity with step_pattern_b_pick).
        self.state.journal = Some(crate::game::Journal::new());

        let opponent_of_active = active.opponent();
        let preview_size_before = self
            .state
            .journal
            .as_ref()
            .map(|j| j.len())
            .unwrap_or(0) as u64;

        let result = self.state.play_card(
            active,
            &picked,
            choices,
            Some(&mut EventContext::new(self.registry.lua(), &mut self.oracle)),
        );

        let suicide = self.state.winner == Some(opponent_of_active);
        let preview_size = self
            .state
            .journal
            .as_ref()
            .map(|j| j.len())
            .unwrap_or(0) as u64;
        bump_preview_attempt(
            &mut self.stats,
            active,
            preview_size.max(preview_size_before),
        );

        if result.is_ok() && !suicide {
            if let Some(mut preview) = self.state.journal.take() {
                if let Some(replay) = self.state.replay_journal.as_mut() {
                    replay.extend_from(&mut preview);
                }
            }
            bump_played(&mut self.stats, active);
            if let Some(card_id) = self
                .state
                .card_pool
                .get(&picked)
                .map(|c| c.card.id.clone())
            {
                match active {
                    PlayerId::A => {
                        self.stats.a_played_card_ids.insert(card_id.clone());
                    }
                    PlayerId::B => {
                        self.stats.b_played_card_ids.insert(card_id.clone());
                    }
                }
                let turn_now = self.state.turn;
                self.stats
                    .card_play_turns
                    .entry(card_id.clone())
                    .and_modify(|(min_t, max_t)| {
                        if turn_now < *min_t {
                            *min_t = turn_now;
                        }
                        if turn_now > *max_t {
                            *max_t = turn_now;
                        }
                    })
                    .or_insert((turn_now, turn_now));
                self.stats
                    .card_play_turn_events
                    .push((card_id, turn_now, active));
            }
            let new_played_creature = played_creature_before || picked_is_creature;
            self.cursor = EngineCursor::PatternBPick {
                played_creature: new_played_creature,
            };
        } else {
            if let Some(journal) = self.state.journal.take() {
                journal.rollback(&mut self.state);
            }
            bump_preview_rollback(&mut self.stats, active);
            if suicide {
                self.state.bump_action("preview_skip_suicide", active);
            }
            // Human-side suicide / failure: don't auto-loop on the
            // same iid (mirror run.rs's `else` branch heuristic).
            if picked_is_creature {
                self.cursor = EngineCursor::PatternBPick {
                    played_creature: true,
                };
            } else {
                self.cursor = EngineCursor::PreCombatActivations;
            }
        }

        StepResult::Continue
    }

    /// S9: AI-side activation pass. `non_creatures_only=true` runs
    /// pre-combat (skips creatures so attack decisions still see them
    /// untapped); `false` runs post-combat (everything still
    /// activatable fires, including vigilant attackers). For the
    /// human-active turn this is a no-op — the human drives
    /// activations explicitly via `HumanAction::Activate` in
    /// Pattern B (S9-extended).
    /// S10: human-active Main2 prompt loop. Yields a `PickCard` with
    /// `state.phase == Main2`; consumes `Pass` / `PlayCard` / `Activate`
    /// (Activate currently re-prompts pending S9-extended). Pass
    /// advances into EndTurn. PlayCard transitions into
    /// `Main2Resolving` and re-dispatches.
    fn step_main2_pick(
        &mut self,
        played_creature: bool,
        pending: Option<HumanAction>,
    ) -> StepResult {
        // Advance Combat → Main2 (idempotent on resume). Fresh oracle
        // per advance matches run_game_continue's RNG order.
        while self.state.phase != Phase::Main2 && self.state.winner.is_none() {
            // We've already passed End somehow → bail to EndTurn.
            if matches!(self.state.phase, Phase::Untap | Phase::Draw) {
                self.cursor = EngineCursor::EndTurn;
                return StepResult::Continue;
            }
            let mut oracle = RandomOracle::new(StdRng::seed_from_u64(self.rng.gen()));
            self.state.next_phase(Some(&mut EventContext::new(
                self.registry.lua(),
                &mut oracle,
            )));
        }
        if self.state.winner.is_some() {
            return StepResult::Continue;
        }
        let active = self.state.active_player;
        let kind_filter = if played_creature {
            PickKindFilter::NonCreatureOnly
        } else {
            PickKindFilter::Any
        };

        // Human dispatch only — AI side never lands in Main2Pick
        // (step_activation_pass routes AI directly to EndTurn).
        match pending {
            None => {
                let candidates = crate::sim::ai::enumerate_playable_in_hand(
                    &self.state,
                    active,
                    kind_filter,
                );
                let activations =
                    crate::sim::run::enumerate_human_activations(&self.state, active);
                let prompt = HumanPrompt::PickCard {
                    state: crate::sim::snapshot::build_state_view(&self.state, active),
                    player: active,
                    candidates,
                    kind_filter,
                    activations,
                };
                StepResult::NeedHuman(Box::new(prompt))
            }
            Some(HumanAction::Pass) => {
                self.cursor = EngineCursor::EndTurn;
                StepResult::Continue
            }
            Some(HumanAction::PlayCard { iid }) => {
                self.cursor = EngineCursor::Main2Resolving {
                    picked: iid,
                    history: Vec::new(),
                    played_creature,
                };
                StepResult::Continue
            }
            Some(HumanAction::Activate { .. }) => {
                // S9-extended (human-side activations); re-prompt for
                // now so the frontend can resend Pass / PlayCard.
                let candidates = crate::sim::ai::enumerate_playable_in_hand(
                    &self.state,
                    active,
                    kind_filter,
                );
                let activations =
                    crate::sim::run::enumerate_human_activations(&self.state, active);
                let prompt = HumanPrompt::PickCard {
                    state: crate::sim::snapshot::build_state_view(&self.state, active),
                    player: active,
                    candidates,
                    kind_filter,
                    activations,
                };
                StepResult::NeedHuman(Box::new(prompt))
            }
            Some(other) => panic!(
                "Main2Pick: expected Pass / PlayCard / Activate response, got {other:?}"
            ),
        }
    }

    /// S10: Main2 resolve sub-state. Identical replay-history protocol
    /// to `step_pattern_b_resolve`; on success the cursor returns to
    /// `Main2Pick` so the human can chain more plays.
    fn step_main2_resolve(
        &mut self,
        picked: InstanceId,
        mut history: Vec<crate::choice::ScriptedAnswer>,
        played_creature: bool,
        pending: Option<HumanAction>,
    ) -> StepResult {
        let active = self.state.active_player;
        if let Some(act) = pending {
            let ans = match act {
                HumanAction::ChoiceCard { iid } => crate::choice::ScriptedAnswer::Card(iid),
                HumanAction::ChoiceConfirm { yes } => {
                    crate::choice::ScriptedAnswer::Confirm(yes)
                }
                HumanAction::ChoicePlayer { player } => {
                    crate::choice::ScriptedAnswer::Player(player)
                }
                HumanAction::ChoiceInt { value } => crate::choice::ScriptedAnswer::Int(value),
                other => panic!("Main2Resolving: expected Choice* response, got {other:?}"),
            };
            history.push(ans);
        }
        self.oracle.clear();
        self.oracle.inner_mut().reset_replay(history.clone());

        let picked_is_creature = self
            .state
            .card_pool
            .get(&picked)
            .map(|c| c.card.kind == CardType::Creature)
            .unwrap_or(false);

        let build_result = build_pattern_b_choices(
            &mut self.state,
            active,
            &picked,
            &mut self.oracle,
            true,
        );
        let choices = match build_result {
            BuildChoiceResult::Choices(c) => c,
            BuildChoiceResult::UnaffordableX { .. } => {
                // Main2: failed build = drop into Main2Pick (allow
                // the player to pick again or Pass). Mirrors
                // PatternBResolving's heuristic but stays in Main2.
                self.cursor = EngineCursor::Main2Pick {
                    played_creature: played_creature || picked_is_creature,
                };
                return StepResult::Continue;
            }
            BuildChoiceResult::Pending(p) => {
                self.cursor = EngineCursor::Main2Resolving {
                    picked,
                    history,
                    played_creature,
                };
                let prompt = pending_to_prompt(&self.state, p);
                return StepResult::NeedHuman(Box::new(prompt));
            }
        };

        for sac_iid in &choices.sacrifice_ids {
            if let Some(card_id) = self
                .state
                .card_pool
                .get(sac_iid)
                .map(|c| c.card.id.clone())
            {
                *self
                    .stats
                    .card_sacrificed_count
                    .entry(card_id)
                    .or_insert(0) += 1;
            }
        }
        self.oracle.clear();
        self.state.journal = Some(crate::game::Journal::new());

        let opponent_of_active = active.opponent();
        let preview_size_before = self
            .state
            .journal
            .as_ref()
            .map(|j| j.len())
            .unwrap_or(0) as u64;

        let result = self.state.play_card(
            active,
            &picked,
            choices,
            Some(&mut EventContext::new(self.registry.lua(), &mut self.oracle)),
        );

        let suicide = self.state.winner == Some(opponent_of_active);
        let preview_size = self
            .state
            .journal
            .as_ref()
            .map(|j| j.len())
            .unwrap_or(0) as u64;
        bump_preview_attempt(
            &mut self.stats,
            active,
            preview_size.max(preview_size_before),
        );

        if result.is_ok() && !suicide {
            if let Some(mut preview) = self.state.journal.take() {
                if let Some(replay) = self.state.replay_journal.as_mut() {
                    replay.extend_from(&mut preview);
                }
            }
            bump_played(&mut self.stats, active);
            if let Some(card_id) = self
                .state
                .card_pool
                .get(&picked)
                .map(|c| c.card.id.clone())
            {
                match active {
                    PlayerId::A => {
                        self.stats.a_played_card_ids.insert(card_id.clone());
                    }
                    PlayerId::B => {
                        self.stats.b_played_card_ids.insert(card_id.clone());
                    }
                }
                let turn_now = self.state.turn;
                self.stats
                    .card_play_turns
                    .entry(card_id.clone())
                    .and_modify(|(min_t, max_t)| {
                        if turn_now < *min_t {
                            *min_t = turn_now;
                        }
                        if turn_now > *max_t {
                            *max_t = turn_now;
                        }
                    })
                    .or_insert((turn_now, turn_now));
                self.stats
                    .card_play_turn_events
                    .push((card_id, turn_now, active));
            }
            self.cursor = EngineCursor::Main2Pick {
                played_creature: played_creature || picked_is_creature,
            };
        } else {
            if let Some(journal) = self.state.journal.take() {
                journal.rollback(&mut self.state);
            }
            bump_preview_rollback(&mut self.stats, active);
            if suicide {
                self.state.bump_action("preview_skip_suicide", active);
            }
            self.cursor = EngineCursor::Main2Pick {
                played_creature: played_creature || picked_is_creature,
            };
        }

        StepResult::Continue
    }

    fn step_activation_pass(&mut self, non_creatures_only: bool) -> StepResult {
        let active = self.state.active_player;
        let mut last_activated: Option<(InstanceId, usize)> = None;
        let _fired = crate::sim::run::run_activation_pass(
            &mut self.state,
            active,
            self.registry.lua(),
            &mut self.oracle,
            non_creatures_only,
            &mut last_activated,
            &self.ais,
        );
        self.cursor = if non_creatures_only {
            EngineCursor::DeclareAttackers
        } else if matches!(self.ais[active.index()], AiKind::Human(_)) {
            // S10: human-active turn routes through Main2 between
            // post-combat activation pass and EndTurn. Main2's own
            // one-creature-per-main-phase counter starts fresh
            // (matches run_game_continue's `m2_played_creature`).
            EngineCursor::Main2Pick {
                played_creature: false,
            }
        } else {
            EngineCursor::EndTurn
        };
        StepResult::Continue
    }

    fn step_declare_blockers(&mut self, pending: Option<HumanAction>) -> StepResult {
        let active = self.state.active_player;
        let defender = active.opponent();
        let assignments = match &self.ais[defender.index()] {
            AiKind::Heuristic | AiKind::Mcts(_) | AiKind::Uct(_) => {
                pick_blocks(&self.state, defender)
            }
            AiKind::Human(_) => match pending {
                None => {
                    use crate::game::CombatState;
                    let declared: Vec<InstanceId> = match &self.state.combat {
                        Some(CombatState::AwaitingBlockers { attacks }) => {
                            attacks.iter().map(|a| a.attacker.clone()).collect()
                        }
                        _ => Vec::new(),
                    };
                    let eligible = eligible_blockers(&self.state, defender);
                    let prompt = HumanPrompt::PickBlocks {
                        state: crate::sim::snapshot::build_state_view(&self.state, defender),
                        defender,
                        attackers: declared,
                        eligible_blockers: eligible,
                    };
                    return StepResult::NeedHuman(Box::new(prompt));
                }
                Some(HumanAction::Blocks { pairs }) => pairs,
                Some(other) => panic!(
                    "DeclareBlockers: expected Blocks response, got {other:?}"
                ),
            },
        };
        for (blk, atk) in &assignments {
            let _ = self
                .state
                .declare_blocker(
                    blk,
                    atk,
                    Some(&mut EventContext::new(self.registry.lua(), &mut self.oracle)),
                );
        }
        let outcome = self
            .state
            .confirm_blocks(Some(&mut EventContext::new(
                self.registry.lua(),
                &mut self.oracle,
            )))
            .unwrap();
        bump_milled(&mut self.stats, defender, outcome.defender_milled_to_exile as u32);
        for death in &outcome.deaths {
            if self.state.card_pool.get(death).map(|i| i.owner) == Some(PlayerId::A) {
                self.stats.a_deaths += 1;
            } else {
                self.stats.b_deaths += 1;
            }
        }
        // Eligibility list for the engine's own bookkeeping.
        let _ = eligible_blockers(&self.state, defender);
        // Post-combat activation pass (S9) runs before EndTurn.
        self.cursor = EngineCursor::PostCombatActivations;
        StepResult::Continue
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardType;
    use crate::cast_routing::CastRouting;

    /// S1 scaffold sanity check. Builds a `StepEngine` over a vanilla
    /// 50-card mirror deck, asserts the cursor begins at `StartTurn`
    /// and the engine state hasn't advanced past turn 1 yet.
    #[test]
    fn step_engine_constructs_at_start_turn() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.kind.is_castable()
            })
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);

        let engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        assert!(
            matches!(engine.cursor, EngineCursor::StartTurn),
            "fresh engine should sit at StartTurn, got {:?}",
            engine.cursor
        );
        assert_eq!(engine.state.turn, 1, "fresh game is on turn 1");
        assert_eq!(
            engine.state.active_player,
            PlayerId::A,
            "side A acts first"
        );
    }

    /// S2 target: full vanilla game (Heuristic-vs-Heuristic, vanilla
    /// 50-card mirror) runs to completion via repeated `step(None)`
    /// calls. Asserts: terminates within a sane step budget, never
    /// yields `NeedHuman` (no humans in this game), produces a
    /// `Done(stats)` with a winner set.
    #[test]
    fn step_engine_completes_vanilla_heuristic_game() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        let mut steps = 0u32;
        let final_stats = loop {
            steps += 1;
            assert!(
                steps < 100_000,
                "step budget exceeded — engine isn't terminating (cursor: {:?})",
                engine.cursor
            );
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(prompt) => {
                    panic!("vanilla Heuristic game should never yield: {prompt:?}")
                }
                StepResult::Done(stats) => break stats,
            }
        };

        assert!(final_stats.turns > 0, "no turns played");
        assert!(
            matches!(engine.cursor, EngineCursor::GameOver),
            "post-Done cursor should be GameOver, got {:?}",
            engine.cursor
        );
    }

    /// S3: byte-for-byte parity vs `run_game_continue` on the same
    /// seed + same decks. If this passes, the step state machine is
    /// observably indistinguishable from the legacy runner for
    /// vanilla games — gives us a safety net for the bigger
    /// refactors (S7+ Lua handlers, S11 edge cases) coming next.
    ///
    /// S2 scope only covers Pattern B + combat, not activations
    /// (those land in S9). The template filter excludes any card
    /// with an `activated` block so `run_game_continue`'s activation
    /// pass and `StepEngine`'s missing pass don't diverge — once S9
    /// adds activation cursors, this filter can drop the
    /// `c.activated.is_empty()` clause.
    #[test]
    fn step_engine_parity_vs_run_game_continue() {
        use crate::game::Journal;
        use crate::sim::run::run_game_continue;
        use rand::SeedableRng;

        let seed: u64 = 0xBEEF;
        let registry_a = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry_a
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.activated.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .unwrap()
            .clone();
        let deck_a_cards: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b_cards = deck_a_cards.clone();

        // Path 1: legacy run_game_continue.
        let mut state1 = GameState::new(deck_a_cards.clone(), deck_b_cards.clone());
        state1.replay_journal = Some(Journal::new());
        let mut rng1 = StdRng::seed_from_u64(seed);
        let mut log1: Vec<String> = Vec::new();
        let ais1 = [AiKind::Heuristic, AiKind::Heuristic];
        let stats1 = run_game_continue(
            &mut state1,
            &mut rng1,
            &mut log1,
            registry_a.lua(),
            &ais1,
        );

        // Path 2: StepEngine. Separate CardRegistry so the Lua VMs
        // can't influence each other (vanilla cards have no handlers
        // so this is belt-and-braces).
        let registry_b = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let mut state2 = GameState::new(deck_a_cards, deck_b_cards);
        state2.replay_journal = Some(Journal::new());
        let mut engine = StepEngine::new(
            state2,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry_b,
            seed,
        );
        let stats2 = engine.run_to_end();

        // Snapshot a few intermediate signals to localize divergence.
        eprintln!(
            "[parity] run_game_continue: winner={:?} turns={} a_played={} b_played={} a_attacks={} b_attacks={} a_milled={} b_milled={}",
            stats1.winner, stats1.turns, stats1.a_played, stats1.b_played,
            stats1.a_attacks, stats1.b_attacks, stats1.a_milled_to_exile, stats1.b_milled_to_exile,
        );
        eprintln!(
            "[parity] StepEngine        : winner={:?} turns={} a_played={} b_played={} a_attacks={} b_attacks={} a_milled={} b_milled={}",
            stats2.winner, stats2.turns, stats2.a_played, stats2.b_played,
            stats2.a_attacks, stats2.b_attacks, stats2.a_milled_to_exile, stats2.b_milled_to_exile,
        );

        assert_eq!(stats1.winner, stats2.winner, "winner differs");
        assert_eq!(stats1.turns, stats2.turns, "turn count differs");
        assert_eq!(stats1.a_played, stats2.a_played, "a_played differs");
        assert_eq!(stats1.b_played, stats2.b_played, "b_played differs");
        assert_eq!(stats1.a_attacks, stats2.a_attacks, "a_attacks differs");
        assert_eq!(stats1.b_attacks, stats2.b_attacks, "b_attacks differs");
        assert_eq!(stats1.a_deaths, stats2.a_deaths, "a_deaths differs");
        assert_eq!(stats1.b_deaths, stats2.b_deaths, "b_deaths differs");
        assert_eq!(stats1.a_final_board, stats2.a_final_board, "a_final_board differs");
        assert_eq!(stats1.b_final_board, stats2.b_final_board, "b_final_board differs");
        assert_eq!(stats1.a_final_gy, stats2.a_final_gy, "a_final_gy differs");
        assert_eq!(stats1.b_final_gy, stats2.b_final_gy, "b_final_gy differs");
        assert_eq!(
            stats1.a_milled_to_exile, stats2.a_milled_to_exile,
            "a_milled_to_exile differs"
        );
        assert_eq!(
            stats1.b_milled_to_exile, stats2.b_milled_to_exile,
            "b_milled_to_exile differs"
        );
        assert_eq!(
            stats1.a_played_card_ids, stats2.a_played_card_ids,
            "a_played_card_ids set differs"
        );
        assert_eq!(
            stats1.b_played_card_ids, stats2.b_played_card_ids,
            "b_played_card_ids set differs"
        );
    }

    /// Template + registry pair for the S4 human-dispatch tests:
    /// vanilla creature with `hand`/`mill`-only cost (no graveyard or
    /// X), no handlers, no activated abilities. Ensures the human
    /// side actually has playable candidates on turn 1.
    fn human_test_setup() -> (CardRegistry, crate::card::Card) {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.activated.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .unwrap()
            .clone();
        (registry, template)
    }

    /// S4: with `AiKind::Human` on side A, the engine yields a
    /// `NeedHuman(PickCard{…})` instead of dispatching the AI picker.
    /// The yielded prompt carries `player=A` and a non-empty
    /// `candidates` list (vanilla mirror deck → A always has hand
    /// cards to play on turn 1).
    #[test]
    fn step_engine_yields_pickcard_for_human_on_pattern_b() {
        use crate::sim::human::{HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        let prompt = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before any human prompt"),
            }
        };
        match *prompt {
            HumanPrompt::PickCard {
                player,
                ref candidates,
                ..
            } => {
                assert_eq!(player, PlayerId::A);
                assert!(!candidates.is_empty(), "vanilla deck should have playables");
            }
            ref other => panic!("expected PickCard, got {other:?}"),
        }
    }

    /// S4: human responds `Pass`, engine advances to `DeclareAttackers`
    /// without playing any cards. Hand size is unchanged.
    #[test]
    fn step_engine_human_pass_advances_to_combat() {
        use crate::sim::human::{HumanAction, HumanInterface};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // Drive to the first PickCard yield.
        loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(_) => break,
                StepResult::Done(_) => panic!("game ended early"),
            }
        }
        let hand_before = engine.state.a.hand.len();
        // Pass: no play, advance into combat (no creatures on board
        // so eventually we wrap through DeclareAttackers → EndTurn).
        match engine.step(Some(HumanAction::Pass)) {
            StepResult::Continue => {}
            other => panic!("expected Continue after Pass, got {other:?}"),
        }
        assert_eq!(
            engine.state.a.hand.len(),
            hand_before,
            "Pass should not consume hand cards"
        );
    }

    /// Pick a creature whose only cost component is an X-cost hand
    /// payment. Hydra fits today (`cost = {{is_x = true, source = "hand"}}`).
    /// Used by the S8 ChooseInt test so we can trigger
    /// `build_pattern_b_choices`'s X-pick yield from a human plays.
    fn x_cost_human_setup() -> (CardRegistry, crate::card::Card) {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.activated.is_empty()
                    && c.cost.len() == 1
                    && c.cost[0].is_x
                    && matches!(c.cost[0].source, crate::card::CostSource::Hand)
            })
            .expect("expected at least one vanilla X-cost-hand creature in the corpus")
            .clone();
        (registry, template)
    }

    /// S8: human plays an X-cost-hand creature. The first oracle call
    /// in `build_pattern_b_choices` is `choose_int` (X-pick). With the
    /// replay queue empty, the engine yields
    /// `NeedHuman(ChooseInt{…})`. Resuming with `ChoiceInt{value}`
    /// drives the X-pick, then the resolve falls through to the
    /// X*hand-payment yield (`ChooseCard`); resuming that too lands
    /// the creature on board.
    #[test]
    fn step_engine_human_x_cost_yields_choose_int_then_choose_card() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = x_cost_human_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // Drive to the first PickCard.
        let prompt = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickCard"),
            }
        };
        let to_play = match *prompt {
            HumanPrompt::PickCard { ref candidates, .. } => candidates[0].clone(),
            ref other => panic!("expected PickCard, got {other:?}"),
        };

        let board_before = engine.state.a.board.len();

        // PlayCard → engine starts resolving → X-cost card → choose_int yield.
        let int_prompt = match engine.step(Some(HumanAction::PlayCard {
            iid: to_play.clone(),
        })) {
            StepResult::NeedHuman(p) => p,
            other => panic!("expected NeedHuman(ChooseInt), got {other:?}"),
        };
        let (min, max) = match *int_prompt {
            HumanPrompt::ChooseInt { min, max, .. } => (min, max),
            ref other => panic!("expected ChooseInt, got {other:?}"),
        };
        assert!(min >= 1, "X-pick min should be ≥ 1, got {min}");
        assert!(max >= min, "X-pick max ≥ min, got {max} vs {min}");

        // Resume with X=1 — minimum payment. Build re-runs, choose_int
        // consumes our reply, then resolve_hand_payment fires once for
        // the single X-slot → choose_card yield.
        let card_prompt = match engine.step(Some(HumanAction::ChoiceInt { value: 1 })) {
            StepResult::NeedHuman(p) => p,
            other => panic!("expected NeedHuman(ChooseCard) after ChoiceInt, got {other:?}"),
        };
        let pool = match *card_prompt {
            HumanPrompt::ChooseCard { ref pool, .. } => pool.clone(),
            ref other => panic!("expected ChooseCard, got {other:?}"),
        };
        assert!(!pool.is_empty(), "X-cost payment pool should be non-empty");

        let payment = pool[0].clone();
        match engine.step(Some(HumanAction::ChoiceCard {
            iid: Some(payment.clone()),
        })) {
            StepResult::Continue => {}
            other => panic!("expected Continue after ChoiceCard, got {other:?}"),
        }

        assert!(
            engine.state.a.board.contains(&to_play),
            "X-cost cast should put the card on A's board"
        );
        assert_eq!(
            engine.state.a.board.len(),
            board_before + 1,
            "board should gain exactly one card"
        );
    }

    /// S7: human plays a 1H creature. `resolve_hand_payment` calls
    /// `oracle.choose_card`; replay queue is empty → engine yields
    /// `NeedHuman(ChooseCard{…})` instead of blocking. Replying with
    /// `ChoiceCard{iid}` resumes the resolve, `play_card` runs, and
    /// the picked card lands on A's board. A's hand drops by two:
    /// one for the card itself, one for the payment.
    #[test]
    fn step_engine_human_playcard_yields_choose_card_for_hand_payment() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        // Template must have a hand cost ≥ 1 for the yield to fire.
        assert!(
            template.cost.iter().any(|c| matches!(
                c.source,
                crate::card::CostSource::Hand
            ) && c.amount >= 1),
            "human_test_setup picked a card with no hand cost — the \
             ChooseCard yield won't fire on this card",
        );
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // Drive to the first PickCard prompt.
        let prompt = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickCard"),
            }
        };
        let to_play = match *prompt {
            HumanPrompt::PickCard { ref candidates, .. } => candidates[0].clone(),
            ref other => panic!("expected PickCard, got {other:?}"),
        };

        let hand_before = engine.state.a.hand.len();
        let board_before = engine.state.a.board.len();

        // Send PlayCard. Build runs, hits resolve_hand_payment, asks
        // `oracle.choose_card`, replay is empty → engine yields
        // ChooseCard back to us.
        let choose_prompt = match engine.step(Some(HumanAction::PlayCard {
            iid: to_play.clone(),
        })) {
            StepResult::Continue => panic!("expected NeedHuman(ChooseCard), got Continue"),
            StepResult::NeedHuman(p) => p,
            StepResult::Done(_) => panic!("game ended unexpectedly"),
        };
        let (pool, asker) = match *choose_prompt {
            HumanPrompt::ChooseCard {
                ref pool,
                asker,
                ..
            } => (pool.clone(), asker),
            ref other => panic!("expected ChooseCard, got {other:?}"),
        };
        assert_eq!(asker, PlayerId::A, "asker should be the human side");
        assert!(!pool.is_empty(), "hand-payment pool must be non-empty");

        // Pick the first eligible iid as the payment.
        let payment = pool[0].clone();

        // Resume: build re-runs with the replay queue [Card(Some(iid))].
        // resolve_hand_payment consumes it; build returns Choices.
        // play_card runs, card moves hand → board.
        match engine.step(Some(HumanAction::ChoiceCard {
            iid: Some(payment.clone()),
        })) {
            StepResult::Continue => {}
            StepResult::NeedHuman(p) => {
                panic!("expected Continue after ChoiceCard, got NeedHuman({p:?})")
            }
            StepResult::Done(_) => panic!("game ended unexpectedly"),
        }

        assert!(
            engine.state.a.board.contains(&to_play),
            "played iid should be on A's board, board={:?}",
            engine.state.a.board
        );
        assert!(
            !engine.state.a.hand.contains(&payment),
            "paid iid should have left A's hand"
        );
        assert_eq!(
            engine.state.a.hand.len(),
            hand_before - 2,
            "hand should drop by 2 (card cast + payment)"
        );
        assert_eq!(
            engine.state.a.board.len(),
            board_before + 1,
            "board should gain exactly the cast card"
        );
    }

    /// S5: with `AiKind::Human` on side A, after the Pattern B pass
    /// the engine yields `NeedHuman(PickAttackers{…})` instead of
    /// running `select_attackers`. Vanilla turn-1 board is empty, so
    /// `eligible` is `[]` — the prompt still fires so the human can
    /// confirm "no attacks".
    #[test]
    fn step_engine_yields_pickattackers_for_human() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // First yield is the Pattern B PickCard (S4). Pass through it.
        let _ = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickCard"),
            }
        };
        // Pass on Pattern B → cursor advances toward DeclareAttackers.
        match engine.step(Some(HumanAction::Pass)) {
            StepResult::Continue => {}
            other => panic!("expected Continue after Pass, got {other:?}"),
        }
        // Drive forward to the next yield — that should be PickAttackers.
        let prompt = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickAttackers"),
            }
        };
        match *prompt {
            HumanPrompt::PickAttackers {
                player,
                ref eligible,
                ..
            } => {
                assert_eq!(player, PlayerId::A);
                assert!(eligible.is_empty(), "turn-1 vanilla board: no creatures yet");
            }
            ref other => panic!("expected PickAttackers, got {other:?}"),
        }
    }

    /// S5: `Attackers{iids: vec![]}` resumes the engine into the
    /// end-of-turn cursor without declaring any attackers.
    #[test]
    fn step_engine_human_attackers_empty_advances_to_endturn() {
        use crate::sim::human::{HumanAction, HumanInterface};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // Drive past PickCard with Pass.
        loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(_) => break,
                StepResult::Done(_) => panic!("game ended early"),
            }
        }
        engine.step(Some(HumanAction::Pass));
        // Drive to PickAttackers.
        loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(_) => break,
                StepResult::Done(_) => panic!("game ended before PickAttackers"),
            }
        }
        // Empty attackers list → PostCombatActivations → (since the
        // active player is the human A) Main2Pick (S10). No EndTurn
        // until the human passes Main2.
        match engine.step(Some(HumanAction::Attackers { iids: vec![] })) {
            StepResult::Continue => {}
            other => panic!("expected Continue after Attackers, got {other:?}"),
        }
        assert!(
            matches!(engine.cursor, EngineCursor::PostCombatActivations),
            "post-Attackers cursor should be PostCombatActivations, got {:?}",
            engine.cursor
        );
        match engine.step(None) {
            StepResult::Continue => {}
            other => panic!("expected Continue from PostCombatActivations, got {other:?}"),
        }
        assert!(
            matches!(engine.cursor, EngineCursor::Main2Pick { .. }),
            "PostCombatActivations should advance into Main2Pick for human-active turn, got {:?}",
            engine.cursor
        );
        assert_eq!(engine.stats.a_attacks, 0, "no attacks bumped");
    }

    /// Drive the engine until a `NeedHuman(prompt)` matching `pick`
    /// fires. Any other NeedHuman (e.g. B's turn-N PickCard) gets a
    /// `Pass` response so the loop keeps making progress. Returns the
    /// matched prompt; panics if the step budget runs out.
    fn drive_to_prompt<F>(engine: &mut StepEngine, mut pick: F) -> Box<HumanPrompt>
    where
        F: FnMut(&HumanPrompt) -> bool,
    {
        use crate::sim::human::{HumanAction, HumanPrompt};
        let mut budget = 5_000u32;
        let mut pending: Option<HumanAction> = None;
        loop {
            budget = budget.checked_sub(1).expect("step budget exhausted");
            match engine.step(pending.take()) {
                StepResult::Continue => {}
                StepResult::Done(_) => panic!("game ended before matching prompt"),
                StepResult::NeedHuman(p) => {
                    if pick(&p) {
                        return p;
                    }
                    // Not the prompt we wanted: pass on PickCard, send
                    // empty Attackers for PickAttackers. Other variants
                    // (PickBlocks, ChooseCard, etc.) are unexpected
                    // inside the drive helper.
                    pending = Some(match *p {
                        HumanPrompt::PickCard { .. } => HumanAction::Pass,
                        HumanPrompt::PickAttackers { .. } => HumanAction::Attackers { iids: vec![] },
                        ref other => panic!(
                            "drive_to_prompt: unexpected intermediate prompt {other:?}"
                        ),
                    });
                }
            }
        }
    }

    /// S5: with `AiKind::Human` on side B (defender) and A=Heuristic
    /// (which Pattern-B-plays creatures that rig + attack via haste),
    /// the engine eventually yields `NeedHuman(PickBlocks{…})` against
    /// B. The `attackers` field of the prompt holds A's declared iids.
    #[test]
    fn step_engine_yields_pickblocks_for_human_defender() {
        use crate::sim::human::{HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Human(Arc::new(iface))],
            registry,
            0xCAFE,
        );

        let prompt = drive_to_prompt(&mut engine, |p| {
            matches!(p, HumanPrompt::PickBlocks { .. })
        });
        match *prompt {
            HumanPrompt::PickBlocks {
                defender,
                ref attackers,
                ..
            } => {
                assert_eq!(defender, PlayerId::B);
                assert!(
                    !attackers.is_empty(),
                    "PickBlocks prompt should carry the declared attackers"
                );
            }
            ref other => panic!("expected PickBlocks, got {other:?}"),
        }
    }

    /// S5: `Blocks{pairs: vec![]}` resumes the engine, runs
    /// `confirm_blocks` (which mills B's deck for the unblocked
    /// attacker), and transitions the cursor into `EndTurn`.
    #[test]
    fn step_engine_human_blocks_empty_advances_to_endturn() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Human(Arc::new(iface))],
            registry,
            0xCAFE,
        );

        let _ = drive_to_prompt(&mut engine, |p| {
            matches!(p, HumanPrompt::PickBlocks { .. })
        });
        let deck_b_before = engine.state.b.deck.len();
        match engine.step(Some(HumanAction::Blocks { pairs: vec![] })) {
            StepResult::Continue => {}
            other => panic!("expected Continue after Blocks, got {other:?}"),
        }
        assert!(
            matches!(engine.cursor, EngineCursor::PostCombatActivations),
            "post-Blocks cursor should be PostCombatActivations, got {:?}",
            engine.cursor
        );
        match engine.step(None) {
            StepResult::Continue => {}
            other => panic!("expected Continue from PostCombatActivations, got {other:?}"),
        }
        assert!(
            matches!(engine.cursor, EngineCursor::EndTurn),
            "PostCombatActivations should advance into EndTurn, got {:?}",
            engine.cursor
        );
        assert!(
            engine.state.b.deck.len() < deck_b_before,
            "unblocked attack should have milled B's deck"
        );
    }

    /// S10: a human-active turn yields a second `PickCard` prompt
    /// after combat — the Main2 main phase. Phase distinguishes it
    /// from the opening Pattern B PickCard: the `state.phase` field
    /// in the prompt is `"Main2"` for this one, `"Main1"` for the
    /// first one of the turn. Frontend uses the same Pass / PlayCard
    /// / Activate action set.
    #[test]
    fn step_engine_yields_main2_pickcard_for_human() {
        use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
        use std::sync::Arc;

        let (registry, template) = human_test_setup();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);
        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();

        let mut engine = StepEngine::new(
            state,
            [AiKind::Human(Arc::new(iface)), AiKind::Heuristic],
            registry,
            0xCAFE,
        );

        // First yield: Pattern B PickCard (Main1). Pass to enter combat.
        let first = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before Main1 PickCard"),
            }
        };
        match *first {
            HumanPrompt::PickCard { ref state, .. } => {
                assert_eq!(
                    state.phase, "Main1",
                    "first PickCard should be in Main1, got phase {:?}",
                    state.phase
                );
            }
            ref other => panic!("expected PickCard, got {other:?}"),
        }
        engine.step(Some(HumanAction::Pass));

        // Next yield: PickAttackers. Empty.
        let attackers = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before PickAttackers"),
            }
        };
        assert!(
            matches!(*attackers, HumanPrompt::PickAttackers { .. }),
            "expected PickAttackers, got {attackers:?}"
        );
        engine.step(Some(HumanAction::Attackers { iids: vec![] }));

        // S10: next yield is Main2 PickCard.
        let second = loop {
            match engine.step(None) {
                StepResult::Continue => continue,
                StepResult::NeedHuman(p) => break p,
                StepResult::Done(_) => panic!("game ended before Main2 PickCard"),
            }
        };
        match *second {
            HumanPrompt::PickCard { ref state, .. } => {
                assert_eq!(
                    state.phase, "Main2",
                    "second PickCard should be in Main2, got phase {:?}",
                    state.phase
                );
            }
            ref other => panic!("expected PickCard, got {other:?}"),
        }
    }

    /// S9: AI-side activation pass fires for cards with activated
    /// abilities on the board. Uses blue-monkey (1H cost, 2H-pay →
    /// draw 1 ability). After a few turns the rig+haste path puts
    /// at least one blue-monkey on each side's board; with hand sizes
    /// at 6+ the AI auto-fires its activation, which calls
    /// `state.bump_action("activate", …)` (the key set by
    /// `state.activate_ability` on successful resolution).
    #[test]
    fn step_engine_runs_ai_activation_pass() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| c.id == "blue-monkey")
            .expect("blue-monkey present in corpus")
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry,
            0xCAFE,
        );
        let _ = engine.run_to_end();

        let total: u32 = engine
            .state
            .action_counts
            .get("activate")
            .map(|v| v[0] + v[1])
            .unwrap_or(0);
        assert!(
            total > 0,
            "AI activation pass should have fired at least once across the game (blue-monkey 2H-pay → draw 1); got total={total}"
        );
    }

    /// GameOver cursor → `Done` repeatedly, no panic. Verifies the
    /// only "real" branch in S1's step() doesn't accidentally
    /// regress when we extend the match in S2+.
    #[test]
    fn step_at_gameover_returns_done() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| matches!(c.kind, CardType::Creature) && c.handlers.is_empty())
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let state = GameState::new(deck_a, deck_b);

        let mut engine = StepEngine::new(
            state,
            [AiKind::Heuristic, AiKind::Heuristic],
            registry,
            0xCAFE,
        );
        engine.cursor = EngineCursor::GameOver;

        match engine.step(None) {
            StepResult::Done(_) => {}
            other => panic!("expected Done at GameOver, got {other:?}"),
        }
        // Second call: still Done (idempotent terminal).
        match engine.step(None) {
            StepResult::Done(_) => {}
            other => panic!("expected Done on second call, got {other:?}"),
        }
    }
}
