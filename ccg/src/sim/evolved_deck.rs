//! Persistent representation of an evolved deck — the artifact you get
//! from a successful EA run and feed back as a gauntlet extra in a
//! subsequent run. Lives on disk as JSON so a human can read, hand-
//! edit, version-control, or delete it.
//!
//! File format (stable, additive-only — never rename or remove a field):
//! ```json
//! {
//!   "label":      "evo1",
//!   "fitness":    1.0,
//!   "base_seed":  60104,
//!   "generations_run": 17,
//!   "card_ids":   ["attach-shuffler", "attach-shuffler", ...]
//! }
//! ```

#![allow(dead_code)]

use std::fs;
use std::path::Path;

use crate::card::{Card, CardRegistry};
use crate::game::DeckUnit;

use super::genome::{to_deck, to_units, GenomeError};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvolvedDeck {
    /// Short human-readable identifier. Used as the per-opponent label
    /// when this deck is loaded as a gauntlet extra. Stem of the
    /// filename is a good default.
    pub label: String,
    /// The fitness recorded by the EA when this deck was the top
    /// genome. Single 140-game observation; the noise floor at n=10
    /// is ~0.043.
    pub fitness: f64,
    /// `cfg.base_seed` from the EA run that produced this deck.
    /// Together with the cfg, the run is reproducible.
    pub base_seed: u64,
    /// Number of generations the EA actually ran (may be less than
    /// `cfg.generations` if early-stop fired).
    pub generations_run: usize,
    /// The 50-card multiset as flat card ids, in genome order.
    pub card_ids: Vec<String>,
}

#[derive(Debug)]
pub enum EvolvedDeckError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Genome(GenomeError),
}

impl std::fmt::Display for EvolvedDeckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvolvedDeckError::Io(e) => write!(f, "I/O error: {e}"),
            EvolvedDeckError::Json(e) => write!(f, "JSON error: {e}"),
            EvolvedDeckError::Genome(e) => write!(f, "genome error: {e}"),
        }
    }
}

impl std::error::Error for EvolvedDeckError {}

impl From<std::io::Error> for EvolvedDeckError {
    fn from(e: std::io::Error) -> Self {
        EvolvedDeckError::Io(e)
    }
}
impl From<serde_json::Error> for EvolvedDeckError {
    fn from(e: serde_json::Error) -> Self {
        EvolvedDeckError::Json(e)
    }
}
impl From<GenomeError> for EvolvedDeckError {
    fn from(e: GenomeError) -> Self {
        EvolvedDeckError::Genome(e)
    }
}

impl EvolvedDeck {
    pub fn save(&self, path: &Path) -> Result<(), EvolvedDeckError> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, EvolvedDeckError> {
        let s = fs::read_to_string(path)?;
        let deck: EvolvedDeck = serde_json::from_str(&s)?;
        Ok(deck)
    }

    /// Materialize the saved card_ids into a `Vec<Card>` ready to be
    /// appended to a gauntlet. Errors if any card id is no longer in
    /// the registry (cards can be renamed or removed between EA runs).
    /// **Legacy — pre-cardless.** Fails on genomes containing the
    /// `__cardless__` sentinel; new call sites should use
    /// [`to_units`](Self::to_units).
    pub fn to_cards(&self, registry: &CardRegistry) -> Result<Vec<Card>, GenomeError> {
        to_deck(registry, &self.card_ids)
    }

    /// Cardless-safe alternative to [`to_cards`](Self::to_cards).
    /// The `__cardless__` sentinel materializes as
    /// [`DeckUnit::Cardless`]; every other id resolves to
    /// [`DeckUnit::Card`]. Errors only on unknown non-sentinel ids.
    pub fn to_units(&self, registry: &CardRegistry) -> Result<Vec<DeckUnit>, GenomeError> {
        to_units(registry, &self.card_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("tsot-evolved-deck-test-{name}.json"));
        p
    }

    #[test]
    fn save_then_load_round_trips() {
        let original = EvolvedDeck {
            label: "evo1".to_string(),
            fitness: 0.986,
            base_seed: 0xEA_C8,
            generations_run: 17,
            card_ids: vec![
                "attach-shuffler".to_string(),
                "attach-shuffler".to_string(),
                "cinder-wurm".to_string(),
            ],
        };
        let path = tmp_path("round_trip");
        original.save(&path).unwrap();
        let loaded = EvolvedDeck::load(&path).unwrap();
        assert_eq!(loaded.label, original.label);
        assert_eq!(loaded.fitness, original.fitness);
        assert_eq!(loaded.base_seed, original.base_seed);
        assert_eq!(loaded.generations_run, original.generations_run);
        assert_eq!(loaded.card_ids, original.card_ids);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn to_cards_resolves_known_ids() {
        let registry = CardRegistry::load(Path::new("cards")).unwrap();
        let first_id = registry.cards()[0].id.clone();
        let deck = EvolvedDeck {
            label: "test".to_string(),
            fitness: 0.5,
            base_seed: 0,
            generations_run: 0,
            card_ids: vec![first_id.clone(), first_id.clone()],
        };
        let cards = deck.to_cards(&registry).unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].id, first_id);
    }
}
