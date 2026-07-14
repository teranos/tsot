//! Replay file format and save/load helpers.
//!
//! Two related artifacts:
//!
//! - `ReplayFile`: seed + deck composition + full journal. Reconstructs a
//!   game from its initial state by applying the journal forward.
//! - `SaveFile`: serialized `GameState` (current snapshot) + optional
//!   replay journal so far. Resumes from any point mid-game.
//!
//! Both use serde + JSON. `Card.handlers` (mlua::Function values) are not
//! serializable — they're skipped at serialization time and re-bound from a
//! live `CardRegistry` via `rebind_handlers` after deserialization.

use crate::card::CardRegistry;
use crate::game::{GameState, Journal};
use serde::{Deserialize, Serialize};

/// Walk a deserialized `GameState`'s `card_pool` and re-attach Lua handlers
/// by looking up each card's id in the live `CardRegistry`. Required after
/// loading from JSON: serialized cards have empty handler maps.
pub fn rebind_handlers(state: &mut GameState, registry: &CardRegistry) -> Result<(), String> {
    for (iid, inst) in &mut state.card_pool {
        let template = registry
            .cards()
            .iter()
            .find(|c| c.id == inst.card().id)
            .ok_or_else(|| {
                format!(
                    "rebind_handlers: card id {:?} (on instance {iid}) not in registry",
                    inst.card().id
                )
            })?;
        inst.card_mut().handlers = template.handlers.clone();
    }
    Ok(())
}

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

/// Mid-game snapshot. Serializes the entire current `GameState` plus any
/// open replay journal so far. Distinct from `ReplayFile`, which carries
/// only the initial setup + full mutation log.
///
/// `cursor` is `Option<_>` so older saves that pre-dated step-engine
/// awareness deserialize cleanly — callers that want to fully resume
/// the StepEngine should use a save whose `cursor` is `Some(_)`.
/// Fields not on this file (ais, rng, stats, log) are reconstructed at
/// load time from caller-provided AI choices + a fresh seed; the
/// trade-off is that rollouts after a load aren't byte-identical to a
/// continuous play, which is acceptable for the developer-facing
/// "send me a savefile, I'll debug it" workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFile {
    pub master_seed: u64,
    pub state: GameState,
    #[serde(default)]
    pub cursor: Option<crate::sim::step::EngineCursor>,
}

impl SaveFile {
    pub fn from_state(state: &GameState, master_seed: u64) -> Self {
        Self {
            master_seed,
            state: state.clone(),
            cursor: None,
        }
    }

    /// Build a save from a live StepEngine. Captures the cursor so
    /// load can place the engine at the same decision point.
    pub fn from_step_engine(
        engine: &crate::sim::step::StepEngine,
        master_seed: u64,
    ) -> Self {
        Self {
            master_seed,
            state: engine.state.clone(),
            cursor: Some(engine.cursor.clone()),
        }
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }

    /// Consume the save and rebuild a live `GameState` with handlers re-bound
    /// against the given registry.
    pub fn restore(mut self, registry: &CardRegistry) -> Result<GameState, String> {
        rebind_handlers(&mut self.state, registry)?;
        Ok(self.state)
    }
}
