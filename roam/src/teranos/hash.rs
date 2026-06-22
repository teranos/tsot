//! Dimensional hashing + weighted-pick primitives.
//!
//! Every independent hash off `(x, y)` used in worldgen gets its own
//! `HashDimension`. The dimension's salt and multiplier are derived
//! from the variant name via FNV-1a-64 at compile time — renaming a
//! dimension reshuffles the world; no hand-typed magic hex literals.
//!
//! `HashDimension` and the hashing internals are `pub(super)` because
//! they're worldgen implementation details — the public surface lives
//! in `flower::flower_at`, `card::card_at`, `terrain::*`.

use super::WORLD_SEED;

// ----- splitmix64 -----

// Named constants from Steele/Lea SplitMix64. These ARE the algorithm —
// changing them produces a different RNG. They live as named consts so
// "0x9E3779B97F4A7C15" never appears as a bare literal in the codebase.
pub const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
pub const SPLITMIX_MIX_1: u64 = 0xBF58_476D_1CE4_E5B9;
pub const SPLITMIX_MIX_2: u64 = 0x94D0_49BB_1331_11EB;

#[inline]
pub(super) fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(SPLITMIX_GAMMA);
    x = (x ^ (x >> 30)).wrapping_mul(SPLITMIX_MIX_1);
    x = (x ^ (x >> 27)).wrapping_mul(SPLITMIX_MIX_2);
    x ^ (x >> 31)
}

// ----- named-derivation hash dimensions -----

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum HashDimension {
    FlowerPresence,
    FlowerPetalEdge,
    FlowerCoreCenter,
    FlowerCoreEdge,
    FlowerPetalCount,
    TerrainBreakNoise,
    TerrainCaveCarve,
    /// Card-presence gate: is there a card on this tile? Independent
    /// of the catalog — same answer every time for the same (x, y).
    CardPresence,
    /// Catalog-index pick: given a non-empty catalog, which entry does
    /// this tile resolve to? Hash → `% catalog.len()`. Changing the
    /// catalog changes which card a tile shows; the *presence* of a
    /// card does not change (CardPresence above). Named CardCatalogIndex
    /// rather than CardSlot to avoid collision with TSOT's `SLOTS.md`
    /// (the 15-region grid on each card face).
    CardCatalogIndex,
}

const FNV_PRIME_64: u64 = 0x0000_0100_0000_01b3;
const FNV_OFFSET_BASIS_SALT: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_OFFSET_BASIS_MULT: u64 = 0xa3b8_c91e_7d6a_2f47;

const fn fnv1a_64(name: &[u8], offset_basis: u64) -> u64 {
    let mut h: u64 = offset_basis;
    let mut i = 0;
    while i < name.len() {
        h ^= name[i] as u64;
        h = h.wrapping_mul(FNV_PRIME_64);
        i += 1;
    }
    h
}

impl HashDimension {
    const fn name(self) -> &'static [u8] {
        match self {
            Self::FlowerPresence => b"FlowerPresence",
            Self::FlowerPetalEdge => b"FlowerPetalEdge",
            Self::FlowerCoreCenter => b"FlowerCoreCenter",
            Self::FlowerCoreEdge => b"FlowerCoreEdge",
            Self::FlowerPetalCount => b"FlowerPetalCount",
            Self::TerrainBreakNoise => b"TerrainBreakNoise",
            Self::TerrainCaveCarve => b"TerrainCaveCarve",
            Self::CardPresence => b"CardPresence",
            Self::CardCatalogIndex => b"CardCatalogIndex",
        }
    }

    pub(super) const fn salt(self) -> u64 {
        fnv1a_64(self.name(), FNV_OFFSET_BASIS_SALT)
    }

    /// OR'd with 1 to guarantee odd — odd multipliers make 64-bit
    /// integer multiply a permutation, which keeps the hash bijective
    /// in its multiplier step.
    pub(super) const fn mult(self) -> u64 {
        fnv1a_64(self.name(), FNV_OFFSET_BASIS_MULT) | 1
    }
}

/// 64-bit hash of (x, y) parametrized by an independent dimension.
/// Same (x, y, dimension) on every peer = same hash, no coordination
/// required. The cylinder seam is folded by canonicalizing x first.
pub(super) fn world_hash(x: i32, y: i32, dim: HashDimension) -> u64 {
    let cx = canonical_x(x);
    splitmix64(
        WORLD_SEED
            ^ dim.salt()
            ^ ((cx as i64 as u64).wrapping_mul(dim.mult()))
            ^ (y as i64 as u64),
    )
}

#[inline]
pub(super) fn canonical_x(x: i32) -> i32 {
    x.rem_euclid(super::WORLD_CIRC_X)
}

// ----- weighted-pick over typed tables -----

/// Sum of a const weight table. Compile-time so the total can be checked
/// against an expected value via `const _: () = assert!(...)`.
pub(super) const fn weight_table_sum<T: Copy, const N: usize>(table: &[(T, u16); N]) -> u32 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < N {
        sum += table[i].1 as u32;
        i += 1;
    }
    sum
}

/// Deterministic weighted pick. `r` is a uniform 64-bit hash; modulo
/// `total` selects an index biased by each row's weight.
///
/// `total` is passed in (not recomputed) because callers pin it as a
/// compile-time constant — the const sum-check on the table catches
/// any drift between declared `total` and actual sum at compile time.
pub(super) fn pick_weighted<T: Copy, const N: usize>(
    r: u64,
    table: &[(T, u16); N],
    total: u32,
) -> T {
    let mut x = (r % total as u64) as u32;
    let mut i = 0;
    while i < N {
        let w = table[i].1 as u32;
        if x < w {
            return table[i].0;
        }
        x -= w;
        i += 1;
    }
    // Unreachable when `total == sum(table)`. The const sum-check on
    // every declared table guarantees this; if we land here it's a
    // logic bug in this function, not a caller-data problem.
    unreachable!("pick_weighted: weight table inconsistent with declared total")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_dimensions_have_distinct_salts() {
        // If two dimensions hash to the same salt, their values are
        // perfectly correlated — a worldgen bug. FNV-1a over distinct
        // variant names should never collide.
        let dims = [
            HashDimension::FlowerPresence,
            HashDimension::FlowerPetalEdge,
            HashDimension::FlowerCoreCenter,
            HashDimension::FlowerCoreEdge,
            HashDimension::FlowerPetalCount,
            HashDimension::TerrainBreakNoise,
            HashDimension::TerrainCaveCarve,
            HashDimension::CardPresence,
            HashDimension::CardCatalogIndex,
        ];
        for i in 0..dims.len() {
            for j in (i + 1)..dims.len() {
                assert_ne!(
                    dims[i].salt(),
                    dims[j].salt(),
                    "salt collision between {:?} and {:?}",
                    dims[i],
                    dims[j]
                );
                assert_ne!(
                    dims[i].mult(),
                    dims[j].mult(),
                    "mult collision between {:?} and {:?}",
                    dims[i],
                    dims[j]
                );
            }
        }
    }

    #[test]
    fn hash_dimension_mult_is_odd() {
        // Odd multiplier keeps the multiply step bijective. The `| 1`
        // in HashDimension::mult guarantees this.
        for dim in [
            HashDimension::FlowerPresence,
            HashDimension::FlowerPetalEdge,
            HashDimension::FlowerCoreCenter,
            HashDimension::FlowerCoreEdge,
            HashDimension::FlowerPetalCount,
            HashDimension::TerrainBreakNoise,
            HashDimension::TerrainCaveCarve,
            HashDimension::CardPresence,
            HashDimension::CardCatalogIndex,
        ] {
            assert_eq!(dim.mult() & 1, 1, "{:?}.mult() must be odd", dim);
        }
    }
}
