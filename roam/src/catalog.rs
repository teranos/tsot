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

use tsot_card::CardId;

/// One entry in the relayer's catalog. Only the fields v0.4 worldgen +
/// inventory display actually need: id + name. Card behavior (Lua
/// source the ccg engine needs at autobattle time) joins this struct
/// when the ccg runtime-load slice lands; until then the wire is
/// minimal so the relayer's first catalog publish is small.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: CardId,
    pub name: String,
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

    pub fn get(&self, id: CardId) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Lookup by position in the published list. The relayer's
    /// canonical order is the order entries arrive; worldgen
    /// hash-picks an index into that order.
    pub fn id_at_index(&self, index: usize) -> Option<CardId> {
        self.entries.get(index).map(|e| e.id)
    }
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
    #[test]
    fn catalog_set_replaces_and_lookup_works() {
        let mut c = Catalog::new();
        c.set(vec![
            CatalogEntry {
                id: CardId(1),
                name: "Appetite".into(),
            },
            CatalogEntry {
                id: CardId(2),
                name: "Salt Wraith".into(),
            },
        ]);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(CardId(1)).map(|e| e.name.as_str()), Some("Appetite"));
        assert_eq!(c.get(CardId(2)).map(|e| e.name.as_str()), Some("Salt Wraith"));
        assert_eq!(c.get(CardId(99)), None);

        // Republish with a smaller list — old entries gone.
        c.set(vec![CatalogEntry {
            id: CardId(7),
            name: "Only".into(),
        }]);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(CardId(1)), None);
        assert_eq!(c.get(CardId(7)).map(|e| e.name.as_str()), Some("Only"));
    }
}
