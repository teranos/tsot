//! Deterministic hash of a chunk / cell coordinate + salt. Duplicated
//! verbatim from `game/src/hash.rs` (same algorithm, same output for the
//! same inputs) so the palette resolver and building/chunk placement
//! stay pure functions of their coordinates without the cdda crate
//! having to depend on game. Determinism is by construction: the
//! function is total and only reads its inputs.

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
