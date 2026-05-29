//! Game-state module: data model, turn flow, zone movement, and card play.
//!
//! Submodules:
//!   - `state`: types and basic accessors (PlayerId, Phase, Zone, CardInstance, GameState, ...).
//!   - `turn`: phase advancement, untap, draw, end-of-turn cleanup.
//!   - `movement`: zone transitions.
//!   - `play`: playing cards from hand, cost payment, attachment.

mod movement;
mod play;
mod state;
mod turn;

#[cfg(test)]
mod test_helpers;

pub use movement::MoveError;
pub use play::{PlayChoices, PlayError};
pub use state::{
    CardInstance, GameState, InstanceId, Modifier, Phase, PlayerId, PlayerState, StatusEffect, Zone,
};
