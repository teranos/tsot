//! Simulation engine. Split into focused submodules:
//! - [`variants`] — variant-pool data still used by `deck_token` and the
//!   `fitness` test fixtures (variant matchup mode has been removed).
//! - [`stats`] — `GameStats` data shape + per-game bump helpers.
//! - [`ai`] — sim AI heuristics.
//! - [`run`] — `run_game`, the per-game turn loop.
//! - [`genome`] / [`ops`] / [`evolve`] / [`fitness`] — EA stack.
//! - [`evolved_deck`] — JSON save/load for evolved decks.
//! - [`deck_token`] — base32 deck-identity tokens (legacy, narrow use).

pub mod ai;
#[cfg(test)]
mod ai_trace_tests;
pub mod deck_presets;
pub mod deck_token;
pub mod diversity;
pub mod evolve;
pub mod evolved_deck;
pub mod fitness;
pub mod game_trace;
pub mod genome;
pub mod human;
pub mod instrument;
pub mod mcts;
pub mod ops;
pub mod palette;
pub mod parallel_eval;
pub mod playable_pool;
pub mod run;
pub mod snapshot;
pub mod stats;
pub mod step;
#[cfg(test)]
mod stress_tests;
pub mod uct;
pub mod variants;

pub use evolve::{evolve as run_evolve, EvolveConfig};
pub use run::{run_game, run_game_with_ai};
pub use stats::GameStats;

/// Which AI drives the sim's player-decision points.
///
/// `Game`, `Fast`, and `Stress` are three intent-named views of ONE
/// shared no-search policy (priority-tier pick, intent-aware targeting,
/// trade-up combat — the picker that's been there since session 1).
/// They are behaviourally identical today; the split exists so each
/// call site declares WHY it wants the policy and so the three can
/// diverge later without a rename:
///
/// - `Game` — the opponent the player actually faces; also the policy
///   MCTS/UCT roll out with, so search estimates reflect real play.
///   The default.
/// - `Fast` — fast, shallow unit/integration tests.
/// - `Stress` — the weekly CI soak/stress harness (wiring TBD).
///
/// When a UCT search is running, this shared picker first consumes the
/// thread-local UCT plan ([`uct::take_planned_action`]) and only falls
/// back to the weighted-random heuristic once the plan is exhausted —
/// a documented property of the picker, not a hidden behaviour of one
/// variant.
///
/// `Mcts` swaps the Pattern B card-pick decision for one-ply rollout
/// MCTS; all other decisions (targets, combat, X-values) stay on the
/// shared policy for v1. `Human` blocks the engine on a channel — the
/// `tsot serve` frontend answers prompts via [`human::HumanInterface`].
#[derive(Debug, Clone, Default)]
pub enum AiKind {
    /// The opponent the player faces; also the rollout policy for
    /// MCTS/UCT search. Today's active behaviour.
    #[default]
    Game,
    /// Fast, shallow tests. Same behaviour as `Game` for now.
    Fast,
    /// Weekly CI stress/soak. Same behaviour as `Game` for now;
    /// reserved so the stress harness can declare its intent.
    Stress,
    Mcts(mcts::MctsConfig),
    /// UCT (UCB1 tree-search) MCTS — persistent tree, UCB1 selection,
    /// expansion at the frontier, heuristic-rollout default policy.
    /// See [`uct::pick_play_uct`]. Cost scales linearly with
    /// `UctConfig.iterations` rather than with `R^depth`.
    Uct(uct::UctConfig),
    Human(std::sync::Arc<human::HumanInterface>),
}
