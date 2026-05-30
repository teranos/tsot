//! Simulation engine for the matchup runner. Splits the per-game loop, the
//! AI heuristics, the per-game data shape, the deck-variant configuration,
//! and the aggregate terminal output into focused submodules.
//!
//! Module boundaries:
//! - [`variants`] — deck-variant config: pools, mandatory pre-fills, card-
//!   level variant exclusivity. Pure data, no game state.
//! - [`stats`] — `GameStats` data shape + per-game bump helpers. Consumed
//!   by both [`run`] (which writes them) and [`aggregate`] (which reads).
//! - [`ai`] — sim AI heuristics: pick-a-playable, play-priority score,
//!   block policy, sacrifice-keep value, etc.
//! - [`run`] — `run_game`, the per-game turn loop tying everything
//!   together. Reads variants/stats, calls ai/* to make decisions.
//! - [`aggregate`] — terminal-format aggregator: prints the matchup matrix,
//!   per-variant aggregate, action totals, etc., to stdout after the run.

pub mod aggregate;
pub mod ai;
pub mod deck_token;
pub mod genome;
pub mod run;
pub mod stats;
pub mod variants;

pub use aggregate::print_aggregate;
pub use deck_token::{DeckToken, Side};
pub use run::run_game;
pub use stats::GameStats;
pub use variants::{
    build_random_deck, mandatory_for_variant, variant_label, variant_pool, DeckVariant, VARIANTS,
};
