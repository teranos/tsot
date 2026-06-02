//! One-ply rollout MCTS for the Pattern B card-pick decision.
//!
//! Wraps `pick_random_playable_in_hand`: enumerate candidates, for each
//! one apply it via `state.play_card` then play out the rest of the
//! game with the heuristic AI as the rollout policy, score by win-rate,
//! pick the candidate with the highest win-rate. Journal-based rollback
//! between rollouts — no `state.clone()` on the hot path.
//!
//! For v1, ONLY the Pattern B card pick is searched. Sub-decisions
//! (target picks via `choose_card`, X-values, attacker/blocker
//! assignment) all use the existing heuristic oracle. Extending MCTS
//! to those decisions is step 7+ in the plan.
//!
//! Determinism contract: same `MctsConfig` + same `state` → same
//! returned candidate. Each rollout derives its RNG seed from
//! `(base_seed, candidate_idx, rollout_idx)` so the rollout sequence
//! is reproducible. Required by the project's "same seed → same
//! outcome" invariant.

#![allow(dead_code)]

use rand::rngs::StdRng;
use rand::SeedableRng;
use tsot::card::CostSource;
use tsot::choice::{RandomOracle, RecordingOracle};
use tsot::game::{EventContext, GameState, InstanceId, Journal, PlayChoices, PlayerId};

use super::ai::{enumerate_playable_in_hand, PickKindFilter};
use super::run::run_game_continue;
use super::AiKind;

#[derive(Debug, Clone)]
pub struct MctsConfig {
    /// Rollouts per candidate. Default 5. At fitness σ ≈ 0.43 the
    /// per-candidate stddev is ~0.18 — enough to separate obvious
    /// good/bad plays. Bump to 20+ for finer comparisons.
    pub rollouts_per_candidate: u32,
    /// Hard cap on candidates considered. Pattern B sees up to ~10
    /// candidates typically; the cap exists as defense against
    /// pathological hands. Above the cap, candidates are truncated
    /// (deterministic — first-N by InstanceId order).
    pub max_candidates: u32,
    /// Base seed for rollout RNG. Each rollout derives its own seed
    /// from `(base_seed, candidate_idx, rollout_idx)`.
    pub base_seed: u64,
}

impl Default for MctsConfig {
    fn default() -> Self {
        Self {
            rollouts_per_candidate: 5,
            max_candidates: 10,
            base_seed: 0xBEEF_FACE,
        }
    }
}

/// One-ply MCTS for "which card to play next in Pattern B?" Returns
/// the highest-win-rate candidate, or `None` if no candidate is
/// playable. Tie-break is deterministic (first-by-InstanceId).
///
/// The rollout policy is the existing heuristic AI — no recursive
/// MCTS. Each rollout opens a fresh journal, applies the candidate,
/// runs `run_game_continue` to completion with `AiKind::Heuristic`,
/// scores the result, then rolls the journal back. After all rollouts
/// the state is byte-identical to the input.
pub fn pick_play(
    state: &mut GameState,
    player: PlayerId,
    kind_filter: PickKindFilter,
    cfg: &MctsConfig,
    lua: &mlua::Lua,
) -> Option<InstanceId> {
    let mut candidates = enumerate_playable_in_hand(state, player, kind_filter);
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return candidates.into_iter().next();
    }
    if (candidates.len() as u32) > cfg.max_candidates {
        candidates.sort();
        candidates.truncate(cfg.max_candidates as usize);
    }

    // Score each candidate by rollout win-rate.
    let mut scored: Vec<(InstanceId, u32, u32)> = Vec::with_capacity(candidates.len());
    for (i, candidate) in candidates.iter().enumerate() {
        let mut wins = 0u32;
        for r in 0..cfg.rollouts_per_candidate {
            let seed = derive_rollout_seed(cfg.base_seed, i as u64, r as u64);
            if simulate_rollout(state, player, candidate, seed, lua) {
                wins += 1;
            }
        }
        scored.push((candidate.clone(), wins, cfg.rollouts_per_candidate));
    }

    // Pick highest win-rate; tie-break by InstanceId for determinism.
    scored.sort_by(|a, b| {
        let a_rate = a.1 as f64 / a.2 as f64;
        let b_rate = b.1 as f64 / b.2 as f64;
        b_rate
            .partial_cmp(&a_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    scored.into_iter().next().map(|(iid, _, _)| iid)
}

/// Derive a rollout-specific seed from (base, candidate_idx, rollout_idx).
/// Splitmix-style mixing — cheap, decent distribution, deterministic.
fn derive_rollout_seed(base: u64, candidate_idx: u64, rollout_idx: u64) -> u64 {
    let mut x = base
        .wrapping_add(candidate_idx.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(rollout_idx.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Run one rollout: apply candidate play, finish the game with the
/// heuristic AI, return true iff `player` won. The state is restored
/// byte-identically before return (journal-based rollback).
fn simulate_rollout(
    state: &mut GameState,
    player: PlayerId,
    candidate: &InstanceId,
    seed: u64,
    lua: &mlua::Lua,
) -> bool {
    // Save the caller's replay_journal aside (typical case: outer
    // game's whole-run capture). We install a fresh journal for the
    // rollout; mutations land in it; we roll it back at the end and
    // restore the original.
    let outer_replay = state.replay_journal.take();
    state.replay_journal = Some(Journal::new());

    let mut rng = StdRng::seed_from_u64(seed);
    let mut log: Vec<String> = Vec::new();
    let mut oracle = RecordingOracle::new(RandomOracle::new(StdRng::seed_from_u64(
        seed.wrapping_add(0xBEEF),
    )));

    // Build default choices for the candidate. Heuristic-ish: pick the
    // first hand card != candidate as the HAND-payment slot. For X-cost
    // we don't search over X — pick X=1 (the min legal). v1 limits
    // search to "which CARD," not "how to play it."
    let choices = build_default_choices(state, player, candidate);

    let cast_ok = state
        .play_card(
            player,
            candidate,
            choices,
            Some(&mut EventContext::new(lua, &mut oracle)),
        )
        .is_ok();

    let won = if cast_ok && state.winner.is_none() {
        // Finish the game with the heuristic AI. CRITICAL: pass
        // AiKind::Heuristic to prevent recursive MCTS inside rollouts
        // (which would explode the search tree).
        let stats = run_game_continue(state, &mut rng, &mut log, lua, &AiKind::Heuristic);
        stats.winner == player
    } else if cast_ok {
        // Cast succeeded but the game ended during play (handler
        // triggered a deck-out, etc.). Whoever's the winner now is it.
        state.winner == Some(player)
    } else {
        // Cast itself failed — treat as a loss for `player` (this
        // candidate is unrollable from here).
        false
    };

    // Roll the rollout's journal back, restore outer state.
    let rollout_journal = state.replay_journal.take().unwrap_or_default();
    rollout_journal.rollback(state);
    state.replay_journal = outer_replay;

    won
}

/// Build a minimal `PlayChoices` for the candidate. v1 strategy:
///   - If the card has no HAND cost: empty payment.
///   - If the card has HAND cost ≥ 1: pick the first hand cards
///     (excluding the candidate itself) as payment.
///   - X-cost: pick X = 1. v1 doesn't search over alternative X values.
///   - Targets / mutations: NOT handled (would need oracle); v1's
///     rollouts use the random oracle internally.
fn build_default_choices(
    state: &GameState,
    player: PlayerId,
    candidate: &InstanceId,
) -> PlayChoices {
    let Some(inst) = state.card_pool.get(candidate) else {
        return PlayChoices::default();
    };
    let mut hand_needed: usize = 0;
    let mut x_value: Option<i32> = None;
    for c in &inst.card.cost {
        if let CostSource::Hand = c.source {
            if c.is_x {
                // Pick X = 1 (the min legal X per RULES P.30 unless the
                // card opts into X = 0; v1 doesn't search alternatives).
                x_value = Some(1);
                hand_needed += 1;
            } else {
                hand_needed += c.amount.max(0) as usize;
            }
        } else if c.is_x && x_value.is_none() {
            x_value = Some(1);
        }
    }
    let mut payment: Vec<InstanceId> = Vec::with_capacity(hand_needed);
    for hid in &state.player(player).hand {
        if payment.len() >= hand_needed {
            break;
        }
        if hid != candidate {
            payment.push(hid.clone());
        }
    }
    PlayChoices {
        hand_payment_ids: payment,
        x_value,
        ..PlayChoices::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsot::card::{CardRegistry, CardType};

    /// Smoke test: MCTS plays a full game vs itself without panicking
    /// and produces a winner. Doesn't measure strength; just confirms
    /// the end-to-end wiring (AiKind plumbing, rollout journal save/
    /// restore, recursion guard) doesn't blow up.
    #[test]
    fn mcts_plays_a_full_game() {
        use crate::sim::{run_game, AiKind};
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

        // Use a tiny MCTS config so the test finishes in seconds.
        let mcts_cfg = MctsConfig {
            rollouts_per_candidate: 2,
            max_candidates: 4,
            base_seed: 0xC0DE,
        };

        // Tweak: run_game itself uses Heuristic (the wrapper hard-codes
        // it). Direct verification is via run_game_continue with the
        // Mcts variant. Use that path here.
        use crate::sim::run::run_game_continue;
        let mut state = state;
        state.replay_journal = Some(Journal::new());
        let mut rng = StdRng::seed_from_u64(0xC0DE);
        let mut log: Vec<String> = Vec::new();
        let stats = run_game_continue(
            &mut state,
            &mut rng,
            &mut log,
            registry.lua(),
            &AiKind::Mcts(mcts_cfg),
        );
        assert!(state.winner.is_some(), "MCTS game produced no winner");
        assert!(stats.turns > 0, "MCTS game recorded zero turns");
        let _ = run_game; // referenced in docs above
    }

    #[test]
    fn pick_play_is_deterministic_per_config() {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry
            .cards()
            .iter()
            .find(|c| matches!(c.kind, CardType::Creature) && c.handlers.is_empty())
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();

        let mut state_1 = GameState::new(deck_a.clone(), deck_b.clone());
        let mut state_2 = GameState::new(deck_a, deck_b);

        // Advance both to a state where Pattern B would pick.
        use tsot::game::Phase;
        for state in [&mut state_1, &mut state_2] {
            while state.phase != Phase::Main1 {
                state.next_phase();
            }
        }

        let cfg = MctsConfig {
            rollouts_per_candidate: 2,
            max_candidates: 5,
            base_seed: 0xC0DE,
        };
        let p1 = pick_play(
            &mut state_1,
            PlayerId::A,
            PickKindFilter::Any,
            &cfg,
            registry.lua(),
        );
        let p2 = pick_play(
            &mut state_2,
            PlayerId::A,
            PickKindFilter::Any,
            &cfg,
            registry.lua(),
        );
        assert_eq!(p1, p2, "MCTS pick must be deterministic per (config, state)");
    }
}
