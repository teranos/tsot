//! Deterministic world-gen hash — the shared primitive behind every
//! procedural placement (trees, campsites, and CDDA-imported buildings
//! later). Same coordinates + same salt → same value on every peer,
//! every session. This is what lets the world be identical for
//! everyone without a shared RNG or a coordination round.
//!
//! Extracted verbatim from the forest placement in `trees.rs` (which
//! ported it from rave) so trees and campsites hash identically.

/// Splitmix64-shaped 32-bit hash of a cell coordinate + salt.
pub fn wang_hash(ix: i32, iz: i32, salt: u32) -> u32 {
    let mut h = (ix as u32)
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add((iz as u32).wrapping_mul(0x85EB_CA77))
        .wrapping_add(salt);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    h
}

/// Deterministic jitter in `[-1.0, 1.0]` for a cell + axis salt, so
/// placements don't read as a regular grid.
pub fn jitter(ix: i32, iz: i32, axis_salt: u32) -> f32 {
    let h = wang_hash(ix, iz, axis_salt.wrapping_mul(0x1234_5678));
    (h as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(wang_hash(3, -7, 0xABCD), wang_hash(3, -7, 0xABCD));
    }

    #[test]
    fn jitter_is_bounded() {
        for ix in -20..20 {
            for iz in -20..20 {
                let j = jitter(ix, iz, 1);
                assert!((-1.0..=1.0).contains(&j), "jitter out of range: {j}");
            }
        }
    }
}
