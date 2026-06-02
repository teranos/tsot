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
pub mod deck_token;
pub mod diversity;
pub mod evolve;
pub mod evolved_deck;
pub mod fitness;
pub mod genome;
pub mod mcts;
pub mod ops;
pub mod parallel_eval;
pub mod run;
pub mod stats;
pub mod variants;

pub use evolve::{evolve as run_evolve, EvolveConfig};
pub use run::run_game;
pub use stats::GameStats;

/// Which AI drives the sim's player-decision points. Default = the
/// heuristic AI that's been there since session 1 (priority-tier
/// pick, intent-aware targeting, trade-up combat). `Mcts` swaps the
/// Pattern B card-pick decision for one-ply rollout MCTS; all other
/// decisions (targets, combat, X-values) stay heuristic for v1.
#[derive(Debug, Clone)]
pub enum AiKind {
    Heuristic,
    Mcts(mcts::MctsConfig),
}

impl Default for AiKind {
    fn default() -> Self {
        AiKind::Heuristic
    }
}
