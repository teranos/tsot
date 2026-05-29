//! Replay file format and dump/load helpers.
//!
//! A `ReplayFile` captures everything needed to deterministically reconstruct
//! a single game: the master seed, the initial deck composition (card ids per
//! player), and the journal of every committed mutation from game start to
//! game end.
//!
//! To replay: rebuild initial `GameState` from the decks (via `CardRegistry`),
//! then call `journal.replay_forward(state)`. The result is byte-identical to
//! the original game's final state.

use crate::card::CardRegistry;
use crate::game::{GameState, Journal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayFile {
    pub seed: u64,
    pub deck_a_card_ids: Vec<String>,
    pub deck_b_card_ids: Vec<String>,
    pub journal: Journal,
}

impl ReplayFile {
    /// Serialize to JSON.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON.
    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }

    /// Reconstruct the initial `GameState` from the decks. Uses the registry
    /// to look up each card id. The returned state is pre-replay (just after
    /// `GameState::new` was called) — call `journal.replay_forward(&mut state)`
    /// to advance to the final state.
    pub fn rebuild_initial_state(&self, registry: &CardRegistry) -> Result<GameState, String> {
        let deck_a = self.lookup_cards(&self.deck_a_card_ids, registry)?;
        let deck_b = self.lookup_cards(&self.deck_b_card_ids, registry)?;
        Ok(GameState::new(deck_a, deck_b))
    }

    fn lookup_cards(
        &self,
        ids: &[String],
        registry: &CardRegistry,
    ) -> Result<Vec<crate::card::Card>, String> {
        ids.iter()
            .map(|id| {
                registry
                    .cards()
                    .iter()
                    .find(|c| c.id == *id)
                    .cloned()
                    .ok_or_else(|| format!("ReplayFile: card id not in registry: {id}"))
            })
            .collect()
    }
}
