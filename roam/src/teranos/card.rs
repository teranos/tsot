//! Card worldgen: two-stage `card_at`.
//!
//! Stage 1 (presence) is local-deterministic — same `(x, y)` always
//! has-or-doesn't-have a card, regardless of catalog state. Stage 2
//! (identity) hashes into the relayer's catalog index space, so card
//! identity follows the catalog. Different relayer = different catalog
//! = different card identities on the same tiles.

pub use tsot_card::CardId;

use super::hash::{world_hash, HashDimension};
use super::terrain::surface_z;
use super::WORLD_Y_LAT;
use crate::catalog::Catalog;

/// Density of cards on land: roughly one card per N candidate tiles.
/// Cards are much rarer than flowers (`FLOWER_DENSITY_DENOM = 600`);
/// the gameplay is "occasionally find a card", not "tiles littered."
pub const CARD_DENSITY_DENOM: u64 = 4_800;

/// Deterministic card at (x, y). Two stages:
///
/// 1. *Presence* — `HashDimension::CardPresence` gate. Doesn't depend
///    on the catalog; same (x, y) → same yes/no every time.
/// 2. *Identity* — `HashDimension::CardSlot` modulo `catalog.len()`
///    picks an entry. If the catalog is empty (relayer hasn't
///    published yet, or is publishing an empty set), there's no card
///    to resolve to — return None even on tiles where stage 1 fires.
///    Once a non-empty catalog is in place, every presence-gated tile
///    resolves to exactly one `CardId`.
pub fn card_at(x: i32, y: i32, catalog: &Catalog) -> Option<CardId> {
    if y.abs() > WORLD_Y_LAT {
        return None;
    }
    if surface_z(x, y) < 0 {
        return None;
    }
    let presence_hash = world_hash(x, y, HashDimension::CardPresence);
    if !presence_hash.is_multiple_of(CARD_DENSITY_DENOM) {
        return None;
    }
    if catalog.is_empty() {
        return None;
    }
    let slot = (world_hash(x, y, HashDimension::CardSlot) % catalog.len() as u64) as usize;
    catalog.id_at_index(slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogEntry;

    #[test]
    fn card_at_deterministic() {
        let cat = Catalog::new();
        let a = card_at(123, -456, &cat);
        let b = card_at(123, -456, &cat);
        assert_eq!(a, b);
    }

    /// Stage-1/stage-2 split: an empty catalog → never any cards even
    /// on tiles where the presence gate fires. Falsifies the regression
    /// where worldgen invents IDs out of nowhere when the relayer's
    /// catalog hasn't arrived.
    #[test]
    fn card_at_returns_none_when_catalog_empty() {
        let cat = Catalog::new();
        for ty in -20..=20 {
            for tx in 0..200 {
                assert!(
                    card_at(tx, ty, &cat).is_none(),
                    "card_at({tx}, {ty}) returned Some on empty catalog"
                );
            }
        }
    }

    /// With a non-empty catalog, presence-gated tiles resolve to a
    /// CardId that's a member of the catalog. Falsifies the regression
    /// where the slot index can pick out-of-range or return a CardId
    /// the catalog doesn't actually contain.
    #[test]
    fn card_at_id_is_always_in_catalog() {
        let mut cat = Catalog::new();
        cat.set(vec![
            CatalogEntry { id: CardId(11), name: "A".into() },
            CatalogEntry { id: CardId(22), name: "B".into() },
            CatalogEntry { id: CardId(33), name: "C".into() },
        ]);
        let mut card_tiles = 0;
        for ty in -20..=20 {
            for tx in 0..400 {
                if let Some(id) = card_at(tx, ty, &cat) {
                    assert!(
                        cat.get(id).is_some(),
                        "card_at returned CardId({}) which isn't in the catalog",
                        id.0
                    );
                    card_tiles += 1;
                }
            }
        }
        assert!(card_tiles > 0, "no card tiles found in scan window — density too rare?");
    }
}
