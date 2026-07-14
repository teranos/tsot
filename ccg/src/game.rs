//! Game-state module: data model, turn flow, zone movement, and card play.
//!
//! Submodules:
//!   - `state`: types and basic accessors (PlayerId, Phase, Zone, Sleeve, GameState, ...).
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

#[cfg(test)]
mod same_sleeve_tests;

#[cfg(test)]
mod cardless_sleeve_tests;

pub use combat::{CombatError, CombatOutcome};
pub use context::EventContext;
pub use journal::{Journal, JournalEntry};
pub use movement::MoveError;
pub use play::{ActivateChoices, ActivateError, PlayChoices, PlayError};
pub use state::{
    AttackDecl, Sleeve, CombatState, GameState, InstanceId, Modifier, Phase, PlayerId,
    PlayerState, PriorityError, PriorityState, StackItem, StatusEffect, Zone,
};

/// Global timeout/spin counter shared across the sim run. Both the
/// response-window spin tripwire (play.rs) and the Pattern B / game
/// watchdog (sim/run.rs) bump it. When the count exceeds
/// `TIMEOUT_HALT_THRESHOLD` the library returns a [`HaltReason`] to
/// its caller; the CLI binary decides whether to `process::exit`. The
/// watchdog scores each individual timed-out game as a loss for the
/// active player, so a run can absorb dozens of slow-card timeouts
/// (dark-salamander, etc.) before the guard fires; the threshold is
/// the regression tripwire, not the per-game accept/reject. Bumped
/// from 5 → 200 after UCT-vs-UCT instrumentation showed 5 was below
/// the normal cost of search-heavy cards on the current pool.
pub static TIMEOUT_COUNTER: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
pub const TIMEOUT_HALT_THRESHOLD: usize = 200;

thread_local! {
    /// Latched halt reason. Set the first time the threshold trips;
    /// read by [`take_halt_reason`] from CLI binaries between games.
    /// Per-thread so parallel EA workers don't trample each other.
    static HALT_REASON_LATCH: std::cell::RefCell<Option<HaltReason>> =
        const { std::cell::RefCell::new(None) };
}

/// Take whichever halt reason the bus latched, leaving `None`. CLI
/// binaries call this between games and `process::exit` if `Some`.
pub fn take_halt_reason() -> Option<HaltReason> {
    HALT_REASON_LATCH.with(|c| c.borrow_mut().take())
}

/// Signal that the global timeout counter has tripped its threshold.
/// Returned by [`bump_timeout_and_maybe_halt`] so the CLI caller (not
/// the library) decides whether to `process::exit`. Library code
/// calling `exit` is a bug — the sim is also driven by the wasm UI
/// where there's no process to exit, and by integration tests that
/// must observe the halt without dying.
#[derive(Debug, Clone)]
pub struct HaltReason {
    pub count: usize,
    pub threshold: usize,
    pub last_site: String,
}

impl std::fmt::Display for HaltReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[HALT] {} game timeouts/spins exceeded threshold ({}). \
             Last site={}. Halting sim — diagnostics in the {} dumps above.",
            self.count, self.threshold, self.last_site, self.count
        )
    }
}

pub fn bump_timeout_and_maybe_halt(site: &str) -> Option<HaltReason> {
    let n = TIMEOUT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    if n > TIMEOUT_HALT_THRESHOLD {
        let reason = HaltReason {
            count: n,
            threshold: TIMEOUT_HALT_THRESHOLD,
            last_site: site.to_string(),
        };
        // Latch so a caller without a return-value channel
        // (drive_window_to_close inside GameState) still has a way
        // to surface the halt without `process::exit`.
        HALT_REASON_LATCH.with(|c| {
            let mut b = c.borrow_mut();
            if b.is_none() {
                *b = Some(reason.clone());
            }
        });
        Some(reason)
    } else {
        None
    }
}
