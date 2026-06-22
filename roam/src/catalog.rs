//! Card catalog as published by the relayer.
//!
//! The relayer is the authority for which cards exist in its world (per
//! the v0.4 design: each relayer governs its own card set, different
//! relayer = different world). roam receives the catalog at connect
//! time and stores it here; worldgen consults this module to resolve
//! card identity at a given tile.
//!
//! This module owns only the *roam-side* of the catalog flow — receive,
//! store, lookup. Catalog transport (gossipsub topic, signature
//! verification, version reconciliation) and catalog publishing
//! (relayer-side) are separate slices.

use serde::{Deserialize, Serialize};
use tsot_card::CardId;

/// One entry in the relayer's catalog. Only the fields v0.4 worldgen +
/// inventory display actually need: id + name. Card behavior (Lua
/// source the ccg engine needs at autobattle time) joins this struct
/// when the ccg runtime-load slice lands; until then the wire is
/// minimal so the relayer's first catalog publish is small.
///
/// Serde shape: `{"id": <u32>, "name": "<string>"}`. The JS bridge
/// forwards bytes from the relayer; `parse_catalog_json` translates an
/// array of these into a `Vec<CatalogEntry>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: CardId,
    pub name: String,
}

/// Parse the relayer's catalog payload as published over the wire.
/// Wire shape: a JSON array of `CatalogEntry` objects. Malformed
/// payloads surface as `Err(message)` — the catalog stays untouched at
/// the call site, no silent fallback to "empty catalog".
pub fn parse_catalog_json(raw: &str) -> Result<Vec<CatalogEntry>, String> {
    serde_json::from_str(raw).map_err(|e| format!("catalog json decode failed: {e}"))
}

/// The set of cards in this world, as the relayer defines it. Owned
/// by `World` (one catalog per session); rebuilt in full whenever the
/// relayer publishes (no partial updates at this layer — the relayer
/// republishes and the new list is the new truth).
#[derive(Debug, Default)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replace the catalog wholesale. Linear scan on lookup is fine
    /// for the catalog size we expect (~250 cards from ccg's corpus,
    /// well within "just iterate"); switch to a `HashMap` if a
    /// profile says otherwise.
    pub fn set(&mut self, entries: Vec<CatalogEntry>) {
        self.entries = entries;
    }

    pub fn get(&self, id: &CardId) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| &e.id == id)
    }

    /// Lookup by position in the published list. The relayer's
    /// canonical order is the order entries arrive; worldgen
    /// hash-picks an index into that order.
    pub fn id_at_index(&self, index: usize) -> Option<&CardId> {
        self.entries.get(index).map(|e| &e.id)
    }

    /// Deterministic color seed for the entry at `index`. Derived from
    /// the card's string id (via FNV-1a) so the render color follows
    /// the card across catalog reorders: same id everywhere → same
    /// seed → same color, regardless of where the entry sits in the
    /// published list.
    pub fn seed_at_index(&self, index: usize) -> Option<u32> {
        self.entries.get(index).map(|e| fnv1a_32(e.id.0.as_bytes()))
    }
}

fn fnv1a_32(bytes: &[u8]) -> u32 {
    const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;
    let mut h: u32 = FNV_OFFSET_BASIS;
    for b in bytes {
        h ^= *b as u32;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsot_card::CardId;

    /// A fresh catalog is empty. Until the relayer's catalog arrives,
    /// roam has no notion of which cards exist; worldgen has to
    /// degrade gracefully (placeholder render) rather than invent
    /// IDs locally.
    #[test]
    fn catalog_starts_empty() {
        let c = Catalog::new();
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
    }

    /// Setting a catalog replaces the previous contents in full. The
    /// relayer publishes one canonical list; partial updates aren't a
    /// thing at this layer (the relayer can republish, and the new
    /// list is the new truth). Lookup by `CardId` returns the matching
    /// entry's name.
    /// JSON wire shape from the relayer: an array of `{id, name}`
    /// objects with string ids matching ccg's card slugs.
    #[test]
    fn parse_catalog_json_round_trip() {
        let json = r#"[{"id":"amsterdam-city","name":"Amsterdam City"},{"id":"anaconda","name":"Anaconda"}]"#;
        let entries = parse_catalog_json(json).expect("valid json must parse");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, CardId("amsterdam-city".into()));
        assert_eq!(entries[0].name, "Amsterdam City");
        assert_eq!(entries[1].id, CardId("anaconda".into()));
        assert_eq!(entries[1].name, "Anaconda");
    }

    /// Errors are sacred. A malformed catalog payload must surface as
    /// an `Err`; the catalog stays untouched at the call site.
    #[test]
    fn parse_catalog_json_rejects_malformed() {
        assert!(parse_catalog_json("not even json").is_err());
        assert!(parse_catalog_json(r#"{"id":"x"}"#).is_err(), "expected array, not single object");
        assert!(
            parse_catalog_json(r#"[{"id":42,"name":"x"}]"#).is_err(),
            "id must be a string, not a number"
        );
    }

    #[test]
    fn catalog_set_replaces_and_lookup_works() {
        let mut c = Catalog::new();
        c.set(vec![
            CatalogEntry {
                id: CardId("amsterdam-city".into()),
                name: "Amsterdam City".into(),
            },
            CatalogEntry {
                id: CardId("anaconda".into()),
                name: "Anaconda".into(),
            },
        ]);
        assert_eq!(c.len(), 2);
        assert_eq!(
            c.get(&CardId("amsterdam-city".into())).map(|e| e.name.as_str()),
            Some("Amsterdam City")
        );
        assert_eq!(
            c.get(&CardId("anaconda".into())).map(|e| e.name.as_str()),
            Some("Anaconda")
        );
        assert_eq!(c.get(&CardId("missing".into())), None);

        // Republish with a smaller list — old entries gone.
        c.set(vec![CatalogEntry {
            id: CardId("apoptosis".into()),
            name: "APOPTOSIS".into(),
        }]);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(&CardId("amsterdam-city".into())), None);
        assert_eq!(
            c.get(&CardId("apoptosis".into())).map(|e| e.name.as_str()),
            Some("APOPTOSIS")
        );
    }

    /// `seed_at_index` is deterministic and `id`-derived: same id →
    /// same seed across catalog reorders. Catches a regression where
    /// the seed is derived from the index (would shift on republish).
    #[test]
    fn catalog_seed_follows_id_across_reorders() {
        let mut a = Catalog::new();
        a.set(vec![
            CatalogEntry { id: CardId("alpha".into()), name: "Alpha".into() },
            CatalogEntry { id: CardId("beta".into()),  name: "Beta".into() },
        ]);
        let alpha_seed = a.seed_at_index(0).unwrap();
        let beta_seed = a.seed_at_index(1).unwrap();
        assert_ne!(alpha_seed, beta_seed);

        // Republish in reverse order.
        let mut b = Catalog::new();
        b.set(vec![
            CatalogEntry { id: CardId("beta".into()),  name: "Beta".into() },
            CatalogEntry { id: CardId("alpha".into()), name: "Alpha".into() },
        ]);
        assert_eq!(b.seed_at_index(0).unwrap(), beta_seed,  "beta now at index 0; seed must still follow beta");
        assert_eq!(b.seed_at_index(1).unwrap(), alpha_seed, "alpha now at index 1; seed must still follow alpha");
    }
}
