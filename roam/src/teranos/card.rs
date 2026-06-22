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

/// Density of cards on land: roughly one card per N candidate tiles.
/// Cards are much rarer than flowers (`FLOWER_DENSITY_DENOM = 600`);
/// the gameplay is "occasionally find a card", not "tiles littered."
pub const CARD_DENSITY_DENOM: u64 = 4_800;

/// Deterministic catalog-index of the card at (x, y), or None. Two
/// stages:
///
/// 1. *Presence* — `HashDimension::CardPresence` gate. Doesn't depend
///    on the catalog; same (x, y) → same yes/no every time.
/// 2. *Index pick* — `HashDimension::CardCatalogIndex` modulo
///    `catalog_len` picks an entry index. If `catalog_len == 0` (relayer
///    hasn't published, or is publishing an empty set), there's no
///    card to resolve to — return None even on tiles where stage 1
///    fires.
///
/// Returns just the index so the renderer can read TileCell bytes
/// without allocating a `String` per frame. Callers that want the
/// actual `CardId` compose via `catalog.id_at_index(idx).cloned()` —
/// see `pickup_at` in the parent module.
pub fn card_at(x: i32, y: i32, catalog_len: usize) -> Option<usize> {
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
    if catalog_len == 0 {
        return None;
    }
    Some((world_hash(x, y, HashDimension::CardCatalogIndex) % catalog_len as u64) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_at_deterministic() {
        let a = card_at(123, -456, 8);
        let b = card_at(123, -456, 8);
        assert_eq!(a, b);
    }

    /// Empty catalog → never any cards even on tiles where the
    /// presence gate fires. Falsifies the regression where worldgen
    /// invents IDs out of nowhere when the relayer's catalog hasn't
    /// arrived.
    #[test]
    fn card_at_returns_none_when_catalog_empty() {
        for ty in -20..=20 {
            for tx in 0..200 {
                assert!(
                    card_at(tx, ty, 0).is_none(),
                    "card_at({tx}, {ty}, 0) returned Some on empty catalog"
                );
            }
        }
    }

    /// Indices always fall in `0..catalog_len`. Falsifies the regression
    /// where the modulus is off-by-one or where presence-gate Some
    /// returns out-of-range index.
    #[test]
    fn card_at_index_is_always_in_range() {
        let len = 3;
        let mut card_tiles = 0;
        for ty in -20..=20 {
            for tx in 0..400 {
                if let Some(idx) = card_at(tx, ty, len) {
                    assert!(idx < len, "card_at returned out-of-range index {idx} >= {len}");
                    card_tiles += 1;
                }
            }
        }
        assert!(card_tiles > 0, "no card tiles found in scan window — density too rare?");
    }
}
