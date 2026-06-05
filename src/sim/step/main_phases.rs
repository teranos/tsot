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

use super::{pending_to_prompt, EngineCursor, StepEngine, StepResult};

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
        // P.34: no per-turn creature cap. The legacy `played_creature`
        // gate forced NonCreatureOnly after the first creature, which
        // is not a real rule — only payment + targeting limit plays.
        // The flag is kept on the cursor so existing callers keep
        // compiling; it's now ignored.
        let _ = played_creature;
        let kind_filter = PickKindFilter::Any;

        // Captured from the UCT arm so the post-match logging block
        // can attach the ASCII tree to engine.log. None for any other
        // AI / human path.
        let mut uct_trace_log: Option<String> = None;
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
                    &self.registry,
                )
            }
            AiKind::Uct(cfg) => {
                let cfg = cfg.clone();
                let (chosen, trace) = crate::sim::uct::pick_play_uct(
                    &mut self.state,
                    active,
                    kind_filter,
                    &cfg,
                    &self.registry,
                );
                let formatted = trace.format_ascii(
                    |iid| {
                        self.state
                            .card_pool
                            .get(iid)
                            .map(|i| i.card.name.clone())
                            .unwrap_or_else(|| iid.to_string())
                    },
                    2,
                );
                uct_trace_log = Some(formatted);
                chosen
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

        // Surface the decision into engine.log so the wasm UI's LOG
        // panel can render it. Cheap (a couple of strings per turn);
        // native CLI gets the same lines for free.
        let actor = match active {
            PlayerId::A => "A",
            PlayerId::B => "B",
        };
        let summary = match &pick {
            Some(iid) => {
                let name = self
                    .state
                    .card_pool
                    .get(iid)
                    .map(|i| i.card.name.clone())
                    .unwrap_or_else(|| iid.to_string());
                format!("turn {} ({}) Main1: play {}", self.state.turn, actor, name)
            }
            None => format!("turn {} ({}) Main1: pass", self.state.turn, actor),
        };
        self.log.push(summary);
        if let Some(t) = uct_trace_log {
            self.log.push(t);
        }

        let Some(picked) = pick else {
            // No more plays this turn; advance into the pre-combat
            // activation pass (S9), then combat.
            self.set_cursor(EngineCursor::PreCombatActivations);
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

        // Order matches run_game_continue: build_pattern_b_choices
        // runs with journal=None so its sacrifice picks etc. are
        // permanent mutations OUTSIDE the preview-rollback envelope.
        // The preview journal opens only for play_card's mutations.
        let build_result = build_pattern_b_choices(
            &mut self.state,
            active,
            &picked,
            &mut self.oracle,
        );
        let choices = match build_result {
            BuildChoiceResult::Choices(c) => c,
            BuildChoiceResult::UnaffordableX { picked_is_creature: pic } => {
                // Same loop-advance as run.rs: if it was a creature,
                // mark played_creature so we don't re-pick it; else
                // bail out of Pattern B and head into combat.
                self.set_cursor(if pic {
                    EngineCursor::PatternBPick {
                        played_creature: true,
                    }
                } else {
                    EngineCursor::PreCombatActivations
                });
                return StepResult::Continue;
            }
            BuildChoiceResult::Pending(p) => {
                // S7: oracle needs the human's answer. Snapshot what
                // we'd need to retry the resolve, transition into the
                // resolving cursor, and yield the prompt. The next
                // step() call will land in `step_pattern_b_resolve`.
                self.set_cursor(EngineCursor::PatternBResolving {
                    picked: picked.clone(),
                    history: Vec::new(),
                    played_creature_before: played_creature,
                });
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
        // No suicide-rescue rewind: both AI and human commit to their
        // played card. If the play causes the active player to lose,
        // they lose. The earlier rescue gate (rolled the oracle's
        // first-player flip and replayed the play for AI-side casts)
        // is gone — it was an asymmetric AI advantage.
        let suicide = self.state.winner == Some(opponent_of_active);
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
            self.set_cursor(EngineCursor::PatternBPick {
                played_creature: new_played_creature,
            });
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
                self.set_cursor(EngineCursor::PatternBPick {
                    played_creature: true,
                });
            } else {
                self.set_cursor(EngineCursor::PreCombatActivations);
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
                self.set_cursor(EngineCursor::EndTurn);
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
        // P.34: no per-turn creature cap. See the matching note in
        // step_pattern_b_pick. `played_creature` is now ignored.
        let _ = played_creature;
        let kind_filter = PickKindFilter::Any;

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
                let actor = match active {
                    PlayerId::A => "A",
                    PlayerId::B => "B",
                };
                self.log.push(format!(
                    "turn {} ({}) Main2: pass",
                    self.state.turn, actor
                ));
                self.set_cursor(EngineCursor::EndTurn);
                StepResult::Continue
            }
            Some(HumanAction::PlayCard { iid }) => {
                let actor = match active {
                    PlayerId::A => "A",
                    PlayerId::B => "B",
                };
                let name = self
                    .state
                    .card_pool
                    .get(&iid)
                    .map(|i| i.card.name.clone())
                    .unwrap_or_else(|| iid.to_string());
                self.log.push(format!(
                    "turn {} ({}) Main2: play {}",
                    self.state.turn, actor, name
                ));
                self.set_cursor(EngineCursor::Main2Resolving {
                    picked: iid,
                    history: Vec::new(),
                    played_creature,
                });
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
        );
        let choices = match build_result {
            BuildChoiceResult::Choices(c) => c,
            BuildChoiceResult::UnaffordableX { .. } => {
                let new_cursor = ctx.on_unaffordable(picked_is_creature);
                self.set_cursor(new_cursor);
                return StepResult::Continue;
            }
            BuildChoiceResult::Pending(p) => {
                let new_cursor = ctx.on_pending(picked.clone(), history);
                self.set_cursor(new_cursor);
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
            let new_cursor = ctx.on_success(picked_is_creature);
            self.set_cursor(new_cursor);
        } else {
            if let Some(journal) = self.state.journal.take() {
                journal.rollback(&mut self.state);
            }
            bump_preview_rollback(&mut self.stats, active);
            if suicide {
                self.state.bump_action("preview_skip_suicide", active);
            }
            let new_cursor = ctx.on_failure(picked_is_creature);
            self.set_cursor(new_cursor);
        }

        StepResult::Continue
    }
}
