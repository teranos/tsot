//! Combat-phase cursor handlers: attacker selection, blocker
//! assignment, and the pre / post-combat activation passes that
//! bracket them. Pulled out of `sim::step::mod` to keep that file
//! readable — see the `EngineCursor` doc-comments in `mod.rs` for the
//! full cursor contract.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::choice::RandomOracle;
use crate::game::{CombatState, EventContext, InstanceId, Phase, PlayerId};
use crate::sim::ai::{eligible_attackers, eligible_blockers, pick_blocks, select_attackers};
use crate::sim::human::{HumanAction, HumanPrompt};
use crate::sim::stats::{bump_attacks, bump_milled};
use crate::sim::AiKind;

use super::{EngineCursor, StepEngine, StepResult};

impl StepEngine {
    pub(super) fn step_declare_attackers(
        &mut self,
        pending: Option<HumanAction>,
    ) -> StepResult {
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
        // Log the attacker selection so the wasm UI's LOG panel can
        // show what the AI (or human) just decided.
        let actor = match active {
            PlayerId::A => "A",
            PlayerId::B => "B",
        };
        if attackers.is_empty() {
            self.log.push(format!(
                "turn {} ({}) Combat: no attackers",
                self.state.turn, actor
            ));
        } else {
            let names: Vec<String> = attackers
                .iter()
                .map(|iid| {
                    self.state
                        .card_pool
                        .get(iid)
                        .map(|i| i.card.name.clone())
                        .unwrap_or_else(|| iid.to_string())
                })
                .collect();
            self.log.push(format!(
                "turn {} ({}) Combat: attack with {}",
                self.state.turn,
                actor,
                names.join(", ")
            ));
        }

        let mut declared_atk_count = 0u32;
        for atk in &attackers {
            match self.state.declare_attacker(
                atk,
                Some(&mut EventContext::new(self.registry.lua(), &mut self.oracle)),
            ) {
                Ok(()) => declared_atk_count += 1,
                Err(e) => {
                    // Sacred-error sweep: silent failure of an
                    // attacker the human picked must surface.
                    let card_name = self
                        .state
                        .card_pool
                        .get(atk)
                        .map(|i| i.card.name.clone())
                        .unwrap_or_else(|| atk.clone());
                    self.emit_human_refusal(
                        active,
                        "prompt",
                        "declare-attackers",
                        format!("Can't attack with {card_name}"),
                        format!("Engine refused declare_attacker: {e:?}"),
                    );
                }
            }
        }
        if declared_atk_count > 0 {
            self.state.confirm_attacks().unwrap();
            self.set_cursor(EngineCursor::DeclareBlockers);
        } else {
            // No attackers declared → skip blockers, still run the
            // post-combat activation pass (run_game_continue runs it
            // unconditionally).
            self.set_cursor(EngineCursor::PostCombatActivations);
        }
        // Eligibility list is consumed; let the compiler know.
        let _ = eligible_attackers(&self.state, active);
        bump_attacks(&mut self.stats, active, declared_atk_count);
        StepResult::Continue
    }

    pub(super) fn step_declare_blockers(
        &mut self,
        pending: Option<HumanAction>,
    ) -> StepResult {
        let active = self.state.active_player;
        let defender = active.opponent();
        let assignments = match &self.ais[defender.index()] {
            AiKind::Heuristic | AiKind::Mcts(_) | AiKind::Uct(_) => {
                pick_blocks(&self.state, defender)
            }
            AiKind::Human(_) => match pending {
                None => {
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
        // Log the block assignment.
        let def_label = match defender {
            PlayerId::A => "A",
            PlayerId::B => "B",
        };
        if assignments.is_empty() {
            self.log.push(format!(
                "turn {} ({}) Combat: no blocks",
                self.state.turn, def_label
            ));
        } else {
            let pairs: Vec<String> = assignments
                .iter()
                .map(|(blk, atk)| {
                    let bn = self
                        .state
                        .card_pool
                        .get(blk)
                        .map(|i| i.card.name.clone())
                        .unwrap_or_else(|| blk.to_string());
                    let an = self
                        .state
                        .card_pool
                        .get(atk)
                        .map(|i| i.card.name.clone())
                        .unwrap_or_else(|| atk.to_string());
                    format!("{} → {}", bn, an)
                })
                .collect();
            self.log.push(format!(
                "turn {} ({}) Combat: block {}",
                self.state.turn,
                def_label,
                pairs.join("; ")
            ));
        }

        for (blk, atk) in &assignments {
            if let Err(e) = self.state.declare_blocker(
                blk,
                atk,
                Some(&mut EventContext::new(self.registry.lua(), &mut self.oracle)),
            ) {
                // Sacred-error sweep: silent block-assignment failure.
                let blk_name = self
                    .state
                    .card_pool
                    .get(blk)
                    .map(|i| i.card.name.clone())
                    .unwrap_or_else(|| blk.clone());
                let atk_name = self
                    .state
                    .card_pool
                    .get(atk)
                    .map(|i| i.card.name.clone())
                    .unwrap_or_else(|| atk.clone());
                self.emit_human_refusal(
                    defender,
                    "prompt",
                    "declare-blockers",
                    format!("Can't block {atk_name} with {blk_name}"),
                    format!("Engine refused declare_blocker: {e:?}"),
                );
            }
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
        self.set_cursor(EngineCursor::PostCombatActivations);
        StepResult::Continue
    }

    /// S9: AI-side activation pass. `non_creatures_only=true` runs
    /// pre-combat (skips creatures so attack decisions still see them
    /// untapped); `false` runs post-combat (everything still
    /// activatable fires, including vigilant attackers). For the
    /// human-active turn this is a no-op — the human drives
    /// activations explicitly via `HumanAction::Activate` in
    /// Pattern B (S9-extended).
    pub(super) fn step_activation_pass(&mut self, non_creatures_only: bool) -> StepResult {
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
        let new_cursor = if non_creatures_only {
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
        self.set_cursor(new_cursor);
        StepResult::Continue
    }
}
