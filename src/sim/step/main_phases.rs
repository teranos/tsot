//! Main-phase cursor handlers — Pattern B (Main1) and Main2 — plus
//! the shared resolve body and `ResolveContext` enum that encodes
//! the cursor-target differences between the two. Pulled out of
//! `sim::step::mod` to keep that file readable.

use crate::card::CardType;
use crate::game::{EventContext, InstanceId};
use crate::sim::ai::{pick_random_playable_in_hand, PickKindFilter};
use crate::sim::human::{HumanAction, HumanPrompt};
use crate::sim::run::{build_pattern_b_choices, BuildChoiceResult};
use crate::sim::stats::{bump_played, bump_preview_attempt, bump_preview_rollback};
use crate::sim::AiKind;

use super::{pending_to_prompt, try_suicide_retry, EngineCursor, PlayAttemptOutcome, StepEngine, StepResult};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::choice::RandomOracle;
use crate::game::{Phase, PlayerId};

/// Resolve-side context for the shared `step_resolve` body — encodes
/// the differences between Pattern B's main-1 resolve and Main2's
/// post-combat resolve. Each variant carries the in-flight
/// `played_creature` flag and computes the four cursor transitions
/// the resolve flow needs: `on_unaffordable`, `on_pending`,
/// `on_success`, `on_failure`.
#[derive(Debug, Clone, Copy)]
enum ResolveContext {
    PatternB { played_creature_before: bool },
    Main2 { played_creature: bool },
}

impl ResolveContext {
    fn panic_label(&self) -> &'static str {
        match self {
            ResolveContext::PatternB { .. } => "PatternBResolving",
            ResolveContext::Main2 { .. } => "Main2Resolving",
        }
    }

    /// Where to land when `build_pattern_b_choices` returns
    /// `UnaffordableX`. Pattern B mirrors `step_pattern_b_pick` —
    /// creature flips the cap and stays in PatternB; non-creature
    /// bails into combat. Main2 always returns to `Main2Pick` with
    /// the merged cap.
    fn on_unaffordable(&self, picked_is_creature: bool) -> EngineCursor {
        match *self {
            ResolveContext::PatternB { .. } => {
                if picked_is_creature {
                    EngineCursor::PatternBPick {
                        played_creature: true,
                    }
                } else {
                    EngineCursor::DeclareAttackers
                }
            }
            ResolveContext::Main2 { played_creature } => EngineCursor::Main2Pick {
                played_creature: played_creature || picked_is_creature,
            },
        }
    }

    /// Where to re-enter the resolve loop when the oracle captures a
    /// `ChoicePending`. Same `*Resolving` variant we came from, with
    /// the accumulated history carried forward.
    fn on_pending(
        &self,
        picked: InstanceId,
        history: Vec<crate::choice::ScriptedAnswer>,
    ) -> EngineCursor {
        match *self {
            ResolveContext::PatternB {
                played_creature_before,
            } => EngineCursor::PatternBResolving {
                picked,
                history,
                played_creature_before,
            },
            ResolveContext::Main2 { played_creature } => EngineCursor::Main2Resolving {
                picked,
                history,
                played_creature,
            },
        }
    }

    /// Where to land after a successful `play_card`. The corresponding
    /// `*Pick` cursor with the cap flag merged in.
    fn on_success(&self, picked_is_creature: bool) -> EngineCursor {
        match *self {
            ResolveContext::PatternB {
                played_creature_before,
            } => EngineCursor::PatternBPick {
                played_creature: played_creature_before || picked_is_creature,
            },
            ResolveContext::Main2 { played_creature } => EngineCursor::Main2Pick {
                played_creature: played_creature || picked_is_creature,
            },
        }
    }

    /// Where to land when `play_card` returned an error (or a future
    /// suicide-detect, currently dormant on the human side). Pattern B
    /// mirrors its pick-fail heuristic; Main2 returns to its pick
    /// cursor so the player can pick again or pass.
    fn on_failure(&self, picked_is_creature: bool) -> EngineCursor {
        match *self {
            ResolveContext::PatternB { .. } => {
                if picked_is_creature {
                    EngineCursor::PatternBPick {
                        played_creature: true,
                    }
                } else {
                    EngineCursor::PreCombatActivations
                }
            }
            ResolveContext::Main2 { played_creature } => EngineCursor::Main2Pick {
                played_creature: played_creature || picked_is_creature,
            },
        }
    }
}

impl StepEngine {
    /// Pattern B: pick a card to play (or pass into combat).
    /// Vanilla AI scope unchanged; the Human arm yields a
    /// `NeedHuman(PickCard{…})` on `pending=None` and resumes from
    /// the human's `HumanAction::Pass` / `PlayCard` on the next call.
    /// `Activate` (S9) currently re-prompts.
    pub(super) fn step_pattern_b_pick(
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

        // S11: clone choices so the rescue helper has them for the
        // flipped-oracle retry. play_card consumes by value.
        let choices_for_retry = choices.clone();
        let resp_before = self
            .state
            .action_counts
            .get("instant_response_played")
            .copied()
            .unwrap_or([0, 0]);

        let initial_result = self.state.play_card(
            active,
            &picked,
            choices,
            Some(&mut EventContext::new(self.registry.lua(), &mut self.oracle)),
        );

        let resp_after = self
            .state
            .action_counts
            .get("instant_response_played")
            .copied()
            .unwrap_or([0, 0]);
        let response_fired =
            resp_after[0] > resp_before[0] || resp_after[1] > resp_before[1];
        let active_is_human = matches!(self.ais[active.index()], AiKind::Human(_));
        let initial_suicide = !active_is_human
            && self.state.winner == Some(opponent_of_active);

        // S11: rescue gate. Only AI-side casts attempt rescue (human
        // owns their decisions). `try_suicide_retry` rolls back the
        // journal, reopens it, replays play_card with a flipped
        // oracle if applicable; returns the final outcome.
        let outcome = if active_is_human {
            PlayAttemptOutcome {
                result: initial_result,
                final_suicide: false, // human casts never auto-rolled
                rescued: false,
            }
        } else {
            let recording: Vec<crate::choice::ScriptedAnswer> =
                self.oracle.recording().to_vec();
            try_suicide_retry(
                &mut self.state,
                active,
                opponent_of_active,
                &picked,
                choices_for_retry,
                initial_result,
                initial_suicide,
                response_fired,
                &recording,
                self.registry.lua(),
            )
        };
        let result = outcome.result;
        let suicide = outcome.final_suicide;
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

    /// S7: resume a human play that previously yielded `NeedHuman`
    /// because the oracle ran out of replay entries. Appends the
    /// freshly-supplied answer to `history`, retries the resolve from
    /// scratch with the full history pre-loaded into the oracle. If
    /// the retry yields again, we stay in this cursor.
    pub(super) fn step_pattern_b_resolve(
        &mut self,
        picked: InstanceId,
        history: Vec<crate::choice::ScriptedAnswer>,
        played_creature_before: bool,
        pending: Option<HumanAction>,
    ) -> StepResult {
        self.step_resolve(
            picked,
            history,
            pending,
            ResolveContext::PatternB {
                played_creature_before,
            },
        )
    }

    /// S10: human-active Main2 prompt loop. Yields a `PickCard` with
    /// `state.phase == Main2`; consumes `Pass` / `PlayCard` /
    /// `Activate` (Activate currently re-prompts pending S9-extended).
    /// Pass advances into EndTurn. PlayCard transitions into
    /// `Main2Resolving` and re-dispatches.
    pub(super) fn step_main2_pick(
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
    pub(super) fn step_main2_resolve(
        &mut self,
        picked: InstanceId,
        history: Vec<crate::choice::ScriptedAnswer>,
        played_creature: bool,
        pending: Option<HumanAction>,
    ) -> StepResult {
        self.step_resolve(
            picked,
            history,
            pending,
            ResolveContext::Main2 { played_creature },
        )
    }

    /// Shared human-side resolve body for `PatternBResolving` and
    /// `Main2Resolving`. The two cursors do the same work — push the
    /// human's response onto the replay history, reset the oracle's
    /// replay, retry `build_pattern_b_choices + play_card`, yield on
    /// captured choices, advance the cursor on success / failure.
    /// `ResolveContext` carries the cursor-target differences (which
    /// `*Pick` to return to, which `*Resolving` to re-enter on yield)
    /// so this single function covers both phases.
    fn step_resolve(
        &mut self,
        picked: InstanceId,
        mut history: Vec<crate::choice::ScriptedAnswer>,
        pending: Option<HumanAction>,
        ctx: ResolveContext,
    ) -> StepResult {
        let active = self.state.active_player;
        let panic_label = ctx.panic_label();
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
                other => panic!(
                    "{panic_label}: expected Choice* response, got {other:?}"
                ),
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
            true, // active_is_human (this helper only fires for human side)
        );
        let choices = match build_result {
            BuildChoiceResult::Choices(c) => c,
            BuildChoiceResult::UnaffordableX { .. } => {
                self.cursor = ctx.on_unaffordable(picked_is_creature);
                return StepResult::Continue;
            }
            BuildChoiceResult::Pending(p) => {
                self.cursor = ctx.on_pending(picked.clone(), history);
                let prompt = pending_to_prompt(&self.state, p);
                return StepResult::NeedHuman(Box::new(prompt));
            }
        };

        // Sacrifice telemetry.
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

        // S11: human-side resolve. Per run_game_continue, human casts
        // are NEVER auto-suicide-rolled-back — the human owns the
        // decision and may legitimately play a card that loses the
        // game. No flip-retry either.
        let suicide = false;
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
            self.cursor = ctx.on_success(picked_is_creature);
        } else {
            if let Some(journal) = self.state.journal.take() {
                journal.rollback(&mut self.state);
            }
            bump_preview_rollback(&mut self.stats, active);
            if suicide {
                self.state.bump_action("preview_skip_suicide", active);
            }
            self.cursor = ctx.on_failure(picked_is_creature);
        }

        StepResult::Continue
    }
}
