//! Game-state module: data model, turn flow, zone movement, and card play.
//!
//! Submodules:
//!   - `state`: types and basic accessors (PlayerId, Phase, Zone, CardInstance, GameState, ...).
//!   - `turn`: phase advancement, untap, draw, end-of-turn cleanup.
//!   - `movement`: zone transitions.
//!   - `play`: playing cards from hand, cost payment, attachment.

mod combat;
mod context;
mod journal;
mod lua_api;
mod movement;
mod play;
mod state;
mod turn;

#[cfg(test)]
pub(crate) mod test_helpers;

#[cfg(test)]
mod trace_tests;

pub use combat::{CombatError, CombatOutcome};
pub use context::EventContext;
pub use journal::{Journal, JournalEntry};
pub use movement::MoveError;
pub use play::{PlayChoices, PlayError};
pub use state::{
    AttackDecl, CardInstance, CombatState, GameState, InstanceId, Modifier, Phase, PlayerId,
    PlayerState, PriorityError, PriorityState, StackItem, StatusEffect, Zone,
};

/// Global timeout/spin counter shared across the sim run. Both the
/// response-window spin tripwire (play.rs) and the Pattern B / game
/// watchdog (sim/run.rs) bump it. When the count exceeds
/// `TIMEOUT_HALT_THRESHOLD`, `bump_and_maybe_halt` calls
/// `std::process::exit(2)` with a loud summary — many timeouts in a
/// single sim run almost always signal a regression, and we'd rather
/// halt loudly than drown stderr in dumps and keep going.
pub static TIMEOUT_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
pub const TIMEOUT_HALT_THRESHOLD: usize = 5;

pub fn bump_timeout_and_maybe_halt(site: &str) {
    let n = TIMEOUT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    if n > TIMEOUT_HALT_THRESHOLD {
        eprintln!(
            "[HALT] {n} game timeouts/spins exceeded threshold ({TIMEOUT_HALT_THRESHOLD}). \
             Last site={site}. Halting sim — diagnostics in the {n} dumps above."
        );
        std::process::exit(2);
    }
}
