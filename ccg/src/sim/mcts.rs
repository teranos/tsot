// S12: tests in this module still construct `run_game_continue` calls
// for parity checks. Production rollouts now state-swap into a
// `StepEngine` instead. Suppress deprecation warnings at the file
// level for the test path.
#![allow(deprecated)]

//! One-ply rollout MCTS for the Pattern B card-pick decision.
//!
//! Wraps `pick_heuristic_playable_in_hand`: enumerate candidates, for each
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

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use rand::rngs::StdRng;
use rand::SeedableRng;
use crate::choice::{RandomOracle, RecordingOracle};
use crate::game::{EventContext, GameState, InstanceId, Journal, PlayerId};

use super::ai::{enumerate_playable_in_hand, pick_heuristic_playable_in_hand, PickKindFilter};
use super::run::{build_pattern_b_choices, BuildChoiceResult};
use super::AiKind;

thread_local! {
    /// Remaining MCTS depth budget for the CURRENT rollout context.
    /// - `0` outside any MCTS search, or after the budget has been
    ///   consumed: subsequent `pick_play` calls degrade to heuristic.
    /// - `>0` while inside a rollout: there are still N more "deeper
    ///   MCTS picks" authorized within this rollout chain.
    ///
    /// Top-level entry to `pick_play` (from `run_game_continue`'s
    /// AiKind dispatch) sees `0` and initializes the search using
    /// `cfg.max_depth`. Each rollout sets the budget to
    /// `depth_for_this_call - 1` before invoking
    /// `run_game_continue`, restoring on exit. So depth=2 fires
    /// MCTS at the top + one deeper MCTS call per rollout chain,
    /// then heuristic for the rest. Cost grows ~R^depth.
    ///
    /// Per-thread so rayon workers don't trample each other.
    static MCTS_BUDGET: Cell<u32> = const { Cell::new(0) };
}

/// Diagnostic counters bumped by `pick_play`. Allow the matchup
/// harness to detect "MCTS was called but never searched" (most
/// often: random-genome decks where Pattern B sees ≤1 candidate
/// per turn, so MCTS short-circuits). Global / process-wide;
/// caller is responsible for resetting before a measurement run.
pub static MCTS_PICK_CALLS: AtomicU64 = AtomicU64::new(0);
pub static MCTS_SEARCHED_PICKS: AtomicU64 = AtomicU64::new(0);
pub static MCTS_TOTAL_CANDIDATES: AtomicU64 = AtomicU64::new(0);

pub fn reset_mcts_diagnostics() {
    MCTS_PICK_CALLS.store(0, Ordering::SeqCst);
    MCTS_SEARCHED_PICKS.store(0, Ordering::SeqCst);
    MCTS_TOTAL_CANDIDATES.store(0, Ordering::SeqCst);
}

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
    /// Search depth — how many `pick_play` calls (in the same rollout
    /// chain) may invoke MCTS before falling back to heuristic.
    /// - `1` = current one-ply behavior (top-level MCTS, rollouts are
    ///   pure heuristic).
    /// - `2` = top-level MCTS + one deeper MCTS pick per rollout, then
    ///   heuristic. Cost ≈ `R^2 + R` finishes per top-level pick.
    /// - `>=3` = exponential cost. Use sparingly.
    ///
    /// Default `1` to preserve current behavior.
    pub max_depth: u32,
}

impl Default for MctsConfig {
    fn default() -> Self {
        Self {
            rollouts_per_candidate: 5,
            max_candidates: 10,
            base_seed: 0xBEEF_FACE,
            max_depth: 1,
        }
    }
}

/// One-ply MCTS for "which card to play next in Pattern B?" Returns
/// the highest-win-rate candidate, or `None` if no candidate is
/// playable. Tie-break is deterministic (first-by-InstanceId).
///
/// The rollout policy is the existing heuristic AI — no recursive
/// MCTS. Each rollout opens a fresh journal, applies the candidate,
/// runs `run_game_continue` to completion with `AiKind::Game`,
/// scores the result, then rolls the journal back. After all rollouts
/// the state is byte-identical to the input.
pub fn pick_play(
    state: &mut GameState,
    player: PlayerId,
    kind_filter: PickKindFilter,
    cfg: &MctsConfig,
    registry: &std::sync::Arc<crate::card::CardRegistry>,
) -> Option<InstanceId> {
    // O6: bracket whole search for AiPick `duration_us`. Cheap
    // no-op when trace is off.
    let trace_active = crate::trace::is_enabled();
    let t0 = trace_active.then(std::time::Instant::now);

    // Resolve the search depth for THIS call. Top-level entry (no
    // budget set) uses `cfg.max_depth`; re-entry from inside a rollout
    // uses whatever budget the outer rollout left for us.
    let entry_budget = MCTS_BUDGET.with(|b| b.get());
    let depth_for_this_call = if entry_budget == 0 {
        cfg.max_depth
    } else {
        entry_budget
    };

    // Out of budget — fall back to the heuristic picker so the rollout
    // (or top-level caller with max_depth=0) keeps making progress.
    if depth_for_this_call == 0 {
        let mut rng = StdRng::seed_from_u64(cfg.base_seed.wrapping_add(0xDEAD_BEEF));
        let chosen = pick_heuristic_playable_in_hand(state, player, &mut rng, kind_filter);
        // Heuristic emits its own AiPick (with ai="Heuristic"); we
        // don't emit a second Mcts-tagged event for this passthrough.
        let _ = t0;
        return chosen;
    }

    MCTS_PICK_CALLS.fetch_add(1, Ordering::SeqCst);
    // Dedup: see `dedup_candidates_by_card_id`. Without it, 6
    // identical iids burn 6× the rollout budget on the same outcome.
    let mut candidates = crate::sim::ai::dedup_candidates_by_card_id(
        state,
        enumerate_playable_in_hand(state, player, kind_filter),
    );
    MCTS_TOTAL_CANDIDATES.fetch_add(candidates.len() as u64, Ordering::SeqCst);
    if candidates.is_empty() {
        emit_mcts_ai_pick(&[], &None, t0);
        return None;
    }
    if candidates.len() == 1 {
        let only = candidates.into_iter().next();
        emit_mcts_ai_pick(
            only.iter().map(|iid| (iid.clone(), 0i32)).collect::<Vec<_>>().as_slice(),
            &only,
            t0,
        );
        return only;
    }
    if (candidates.len() as u32) > cfg.max_candidates {
        candidates.sort();
        candidates.truncate(cfg.max_candidates as usize);
    }
    MCTS_SEARCHED_PICKS.fetch_add(1, Ordering::SeqCst);

    // Score each candidate by rollout win-rate. Each rollout runs with
    // the budget set to `depth_for_this_call - 1`, so the rollout's
    // first recursive `pick_play` (if depth > 1) does deeper MCTS and
    // subsequent picks degrade to heuristic.
    let rollout_budget = depth_for_this_call.saturating_sub(1);
    let mut scored: Vec<(InstanceId, u32, u32)> = Vec::with_capacity(candidates.len());
    for (i, candidate) in candidates.iter().enumerate() {
        let mut wins = 0u32;
        for r in 0..cfg.rollouts_per_candidate {
            let seed = derive_rollout_seed(cfg.base_seed, i as u64, r as u64);
            MCTS_BUDGET.with(|b| b.set(rollout_budget));
            let won = simulate_rollout(state, player, candidate, seed, registry, cfg);
            MCTS_BUDGET.with(|b| b.set(entry_budget));
            if won {
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
    let chosen = scored.first().map(|(iid, _, _)| iid.clone());
    // O6: emit AiPick. Each candidate's score = wins (rollouts that
    // won out of `rollouts_per_candidate`). Trace consumer can
    // compute win-rate by dividing by `rollouts_per_candidate`.
    let candidate_scores: Vec<(InstanceId, i32)> = scored
        .iter()
        .map(|(iid, wins, _)| (iid.clone(), *wins as i32))
        .collect();
    emit_mcts_ai_pick(&candidate_scores, &chosen, t0);
    chosen
}

/// O6: shared AiPick emission for `pick_play` (MCTS). Same shape
/// as the UCT helper; tagged `ai = "Mcts"`. Scores are rollout
/// win counts (out of `cfg.rollouts_per_candidate`).
fn emit_mcts_ai_pick(
    scored: &[(InstanceId, i32)],
    chosen: &Option<InstanceId>,
    t0: Option<std::time::Instant>,
) {
    let Some(t0) = t0 else { return };
    let candidates: Vec<crate::trace::CandidateScore> = scored
        .iter()
        .map(|(iid, score)| crate::trace::CandidateScore {
            iid: iid.clone(),
            score: *score,
            rejected_reason: None,
        })
        .collect();
    crate::trace::push(crate::trace::TraceEvent::AiPick {
        at_us: crate::trace::now_us(),
        ai: "Mcts".to_string(),
        candidates,
        chosen: chosen.clone(),
        duration_us: t0.elapsed().as_micros() as u64,
    });
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

/// Run one rollout: apply candidate play, finish the game with either
/// pure heuristic (when no MCTS budget remains) or with MCTS dispatched
/// at the next `pick_play` boundary (when there's depth budget). The
/// state is restored byte-identically before return.
fn simulate_rollout(
    state: &mut GameState,
    player: PlayerId,
    candidate: &InstanceId,
    seed: u64,
    registry: &std::sync::Arc<crate::card::CardRegistry>,
    cfg: &MctsConfig,
) -> bool {
    let lua = registry.lua();
    // Save the caller's replay_journal aside (typical case: outer
    // game's whole-run capture). We install a fresh journal for the
    // rollout; mutations land in it; we roll it back at the end and
    // restore the original.
    let outer_replay = state.replay_journal.take();
    state.replay_journal = Some(Journal::new());

    let mut oracle = RecordingOracle::new(RandomOracle::new(StdRng::seed_from_u64(
        seed.wrapping_add(0xBEEF),
    )));

    // CRITICAL: use the same choice-builder Pattern B uses. Without
    // this, MCTS rollouts pay for cards with a stripped-down "first-N
    // hand cards" heuristic while real Pattern B uses smart-pitch +
    // jewel-tap + Clear-View. Asymmetric choice quality systematically
    // makes MCTS underestimate any candidate with non-trivial cost,
    // and MCTS picks worse than heuristic.
    let choices = match build_pattern_b_choices(state, player, candidate, &mut oracle) {
        BuildChoiceResult::Choices(c) => c,
        BuildChoiceResult::UnaffordableX { .. } => {
            // Candidate can't be paid for; treat as a loss for `player`.
            let rollout_journal = state.replay_journal.take().unwrap_or_default();
            rollout_journal.rollback(state);
            state.replay_journal = outer_replay;
            return false;
        }
        BuildChoiceResult::Pending(p) => {
            // MCTS rollouts use RandomOracle which never returns
            // ChoicePending. Treat as a loss for `player` to bail out
            // cleanly if a future oracle wraps this path.
            //
            // Sacred-error: if this fires, the "RandomOracle never
            // returns Pending" invariant is broken — surface it as
            // a typed Error so the bug isn't invisible in wasm UCT.
            crate::error::emit_region(
                crate::error::Severity::Error,
                "mcts",
                "unexpected-pending",
                "MCTS rollout received ChoicePending; RandomOracle should never produce one".to_string(),
                format!("{p:?}"),
            );
            eprintln!("MCTS rollout: unexpected ChoicePending: {p:?}");
            let rollout_journal = state.replay_journal.take().unwrap_or_default();
            rollout_journal.rollback(state);
            state.replay_journal = outer_replay;
            return false;
        }
    };

    let cast_ok = state
        .play_card(
            player,
            candidate,
            choices,
            Some(&mut EventContext::new(lua, &mut oracle)),
        )
        .is_ok();

    let won = if cast_ok && state.winner.is_none() {
        // Finish the game. If MCTS_BUDGET > 0 (multi-ply with depth
        // remaining), use [Mcts, Mcts] so the next `pick_play` does
        // deeper search. The thread-local budget decrements per
        // recursive call and bottoms out at heuristic — so this
        // doesn't explode the search tree.
        let budget = MCTS_BUDGET.with(|b| b.get());
        let ais = if budget > 0 {
            [AiKind::Mcts(cfg.clone()), AiKind::Mcts(cfg.clone())]
        } else {
            [AiKind::Game, AiKind::Game]
        };
        // S12: state-swap MCTS rollout finish into a StepEngine.
        // Swap state out of the caller's `&mut`, hand it to the
        // engine by value, let the engine drive to Done, then swap
        // the mutated final state back. The journal we opened above
        // travels with the state, so rollback still works.
        let placeholder = crate::game::GameState::new(Vec::new(), Vec::new());
        let taken = std::mem::replace(state, placeholder);
        let rollout_seed = seed.wrapping_add(0xF1F1_F1F1);
        let mut engine =
            crate::sim::step::StepEngine::new(taken, ais, registry.clone(), rollout_seed);
        // O6 fix: suspend trace bus during the rollout. See
        // matching note in `pick_play_uct`.
        let stats = crate::trace::suspend(|| engine.run_to_end());
        *state = engine.state;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{CardRegistry, CardType};

    /// Smoke test: MCTS plays a full game vs itself without panicking
    /// and produces a winner. Doesn't measure strength; just confirms
    /// the end-to-end wiring (AiKind plumbing, rollout journal save/
    /// restore, recursion guard) doesn't blow up.
    #[test]
    fn mcts_plays_a_full_game() {
        use crate::sim::{run_game, AiKind};
        let registry = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
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
            max_depth: 1,
        };

        // Tweak: run_game itself uses Heuristic (the wrapper hard-codes
        // it). Direct verification is via run_game_continue with the
        // Mcts variant. Use that path here.
        use crate::sim::run::run_game_continue;
        let mut state = state;
        state.replay_journal = Some(Journal::new());
        let mut rng = StdRng::seed_from_u64(0xC0DE);
        let mut log: Vec<String> = Vec::new();
        // MCTS on both sides; the smoke test just verifies the rollout
        // wiring + recursion guard works.
        let ais = [AiKind::Mcts(mcts_cfg.clone()), AiKind::Mcts(mcts_cfg)];
        let stats = run_game_continue(
            &mut state,
            &mut rng,
            &mut log,
            &registry,
            &ais,
            0xC0DE,
        );
        assert!(state.winner.is_some(), "MCTS game produced no winner");
        assert!(stats.turns > 0, "MCTS game recorded zero turns");
        let _ = run_game; // referenced in docs above
    }

    #[test]
    fn pick_play_is_deterministic_per_config() {
        let registry = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
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
        use crate::game::Phase;
        for state in [&mut state_1, &mut state_2] {
            while state.phase != Phase::Main1 {
                state.next_phase(None).expect("None ctx never yields");
            }
        }

        let cfg = MctsConfig {
            rollouts_per_candidate: 2,
            max_candidates: 5,
            base_seed: 0xC0DE,
            max_depth: 1,
        };
        let p1 = pick_play(
            &mut state_1,
            PlayerId::A,
            PickKindFilter::Any,
            &cfg,
            &registry,
        );
        let p2 = pick_play(
            &mut state_2,
            PlayerId::A,
            PickKindFilter::Any,
            &cfg,
            &registry,
        );
        assert_eq!(p1, p2, "MCTS pick must be deterministic per (config, state)");
    }
}
